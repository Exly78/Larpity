//! The WebSocket endpoints. Each connection gets a reader loop (this task) and a
//! writer task, joined by an mpsc channel; the writer also sends the keep-alive pings.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use bytes::Bytes;
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::Deserialize;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
    time::{Instant, interval_at, timeout},
};
use tower_http::services::ServeDir;
use tracing::{debug, info, warn};

use crate::{
    ai, chat,
    config::Config,
    hub::{Hub, Pick, Speech, ViewerEvent},
    protocol::{AiMessage, ErrorCode, FrameHeader, FrameKind, HEADER_LEN, PhoneMessage, ServerMessage, ViewerMessage},
    recording::Recorder,
};

/// Keyframes at 720p can be a few hundred KB; leave plenty of room.
const PHONE_MAX_MESSAGE: usize = 4 << 20;
/// Viewers only send small JSON.
const VIEWER_MAX_MESSAGE: usize = 4 << 10;
/// Speech arrives as up to 2 MB of audio in base64.
const AI_MAX_MESSAGE: usize = 4 << 20;
const OUTBOX: usize = 64;
/// The AI host gets the chat history on connect, then every message.
const AI_OUTBOX: usize = 1024;

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<Hub>,
    pub config: Arc<Config>,
}

pub fn router(config: Config) -> Router {
    let client = ServeDir::new(&config.client_dir);
    let firmware = ServeDir::new(&config.firmware_dir);
    let state = AppState {
        hub: Arc::new(Hub::new()),
        config: Arc::new(config),
    };
    Router::new()
        .route("/ws/phone", get(phone_ws))
        .route("/ws/viewer", get(viewer_ws))
        .route("/ws/ai", get(ai_ws))
        .route("/health", get(|| async { "ok" }))
        .route("/status", get(status))
        // Without the slash, relative links on the phone page would resolve against `/`.
        .route("/phone", get(|| async { Redirect::permanent("/phone/") }))
        .nest_service("/phone/", firmware)
        .fallback_service(client)
        .with_state(state)
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Is it working? Phone, stream, viewers, AI host, questions and recording, as JSON.
async fn status(State(state): State<AppState>) -> Response {
    let no_store = axum::http::HeaderValue::from_static("no-store");
    (
        [(axum::http::header::CACHE_CONTROL, no_store)],
        axum::Json(state.hub.status()),
    )
        .into_response()
}

async fn phone_ws(State(state): State<AppState>, Query(query): Query<TokenQuery>, ws: WebSocketUpgrade) -> Response {
    if !state.config.authorized(query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.max_message_size(PHONE_MAX_MESSAGE)
        .on_upgrade(move |socket| run_phone(state, socket))
}

async fn viewer_ws(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(VIEWER_MAX_MESSAGE)
        .on_upgrade(move |socket| run_viewer(state, socket))
}

async fn ai_ws(State(state): State<AppState>, Query(query): Query<TokenQuery>, ws: WebSocketUpgrade) -> Response {
    if !state.config.authorized(query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.max_message_size(AI_MAX_MESSAGE)
        .on_upgrade(move |socket| run_ai(state, socket))
}

async fn run_phone(state: AppState, socket: WebSocket) {
    let (sink, stream) = socket.split();
    let (tx, rx) = mpsc::channel(OUTBOX);
    let session = state.hub.attach_phone(tx.clone());
    info!(session, "phone connected");
    let writer = tokio::spawn(write_loop(sink, rx, state.config.ping_every));
    let mut inbound = Inbound::new(stream, state.config.idle_timeout);
    let mut configured = false;
    let mut warned_unconfigured = false;
    let mut recorder: Option<Recorder> = None;

    while let Some(message) = inbound.next().await {
        if !state.hub.is_phone(session) {
            break;
        }
        match message {
            Message::Text(text) => match serde_json::from_str::<PhoneMessage>(&text) {
                Ok(PhoneMessage::Config(config)) => {
                    info!(session, ?config, "stream config");
                    state.hub.set_config(session, &config);
                    configured = true;
                    if let Some(dir) = &state.config.record_dir {
                        recorder = match Recorder::start(dir, &config).await {
                            Ok(rec) => {
                                info!(session, path = %rec.base().display(), "recording");
                                Some(rec)
                            }
                            Err(err) => {
                                warn!(session, %err, "can't start a recording");
                                None
                            }
                        };
                        state
                            .hub
                            .set_recording(session, recorder.as_ref().map(|rec| rec.base().to_path_buf()));
                    }
                }
                Ok(PhoneMessage::Button) => match state.hub.pick_question(session, state.config.button_debounce) {
                    Pick::Picked => info!(session, "button: question picked"),
                    Pick::Empty => send_error(&tx, ErrorCode::NoQuestion),
                    Pick::Ignored => debug!(session, "button: ignored repeat press"),
                },
                Err(err) => {
                    debug!(session, %err, "bad phone message");
                    send_error(&tx, ErrorCode::BadMessage);
                }
            },
            Message::Binary(frame) => {
                let Some(header) = FrameHeader::parse(&frame) else {
                    send_error(&tx, ErrorCode::BadFrame);
                    continue;
                };
                if !configured {
                    if !warned_unconfigured {
                        send_error(&tx, ErrorCode::ConfigRequired);
                        warned_unconfigured = true;
                    }
                    continue;
                }
                match header.kind {
                    FrameKind::Video => state.hub.publish_video(session, frame.clone(), header.keyframe),
                    FrameKind::Audio => state.hub.publish_audio(session, frame.clone()),
                    FrameKind::Speech => {
                        send_error(&tx, ErrorCode::BadFrame);
                        continue;
                    }
                }
                let was_recording = recorder.is_some();
                record(&mut recorder, header.kind, &frame[HEADER_LEN..]).await;
                if was_recording && recorder.is_none() {
                    state.hub.set_recording(session, None);
                }
            }
            _ => {}
        }
    }

    state.hub.detach_phone(session);
    info!(session, "phone disconnected");
    drop(tx);
    finish(writer).await;
}

async fn run_viewer(state: AppState, socket: WebSocket) {
    let (sink, stream) = socket.split();
    let (tx, rx) = mpsc::channel::<Message>(OUTBOX);
    let (snapshot, events) = state.hub.join_viewer();
    info!(viewers = state.hub.viewer_count(), "viewer connected");
    let writer = tokio::spawn(viewer_write_loop(sink, snapshot, events, rx, state.config.ping_every));
    let mut inbound = Inbound::new(stream, state.config.idle_timeout);
    let mut nick: Option<String> = None;
    let mut last_chat: Option<Instant> = None;

    while let Some(message) = inbound.next().await {
        let Message::Text(text) = message else {
            send_error(&tx, ErrorCode::BadMessage);
            continue;
        };
        match serde_json::from_str::<ViewerMessage>(&text) {
            Ok(ViewerMessage::Join { nick: wanted }) => {
                let Some(wanted) = chat::clean_nick(&wanted) else {
                    send_error(&tx, ErrorCode::BadNick);
                    continue;
                };
                let given = state.hub.claim_nick(&wanted, nick.as_deref());
                let _ = tx.try_send(Message::Text(ServerMessage::Joined { nick: &given }.to_text()));
                nick = Some(given);
            }
            Ok(ViewerMessage::Chat { text }) => {
                let Some(nick) = &nick else {
                    send_error(&tx, ErrorCode::JoinRequired);
                    continue;
                };
                let Some(text) = chat::clean_text(&text) else {
                    send_error(&tx, ErrorCode::BadText);
                    continue;
                };
                if last_chat.is_some_and(|at| at.elapsed() < state.config.chat_interval) {
                    send_error(&tx, ErrorCode::RateLimited);
                    continue;
                }
                last_chat = Some(Instant::now());
                state.hub.post_chat(nick, text);
            }
            Err(_) => send_error(&tx, ErrorCode::BadMessage),
        }
    }

    if let Some(nick) = nick {
        state.hub.release_nick(&nick);
    }
    drop(tx);
    finish(writer).await;
    info!(viewers = state.hub.viewer_count(), "viewer disconnected");
}

async fn run_ai(state: AppState, socket: WebSocket) {
    let (sink, stream) = socket.split();
    let (tx, rx) = mpsc::channel(AI_OUTBOX);
    let session = state.hub.attach_ai(tx.clone());
    info!(session, "AI host connected");
    let writer = tokio::spawn(write_loop(sink, rx, state.config.ping_every));
    let mut inbound = Inbound::new(stream, state.config.idle_timeout);

    while let Some(message) = inbound.next().await {
        if !state.hub.is_ai(session) {
            break;
        }
        let Message::Text(text) = message else {
            send_error(&tx, ErrorCode::BadMessage);
            continue;
        };
        match serde_json::from_str::<AiMessage>(&text) {
            Ok(AiMessage::Ranking { questions }) => {
                let (questions, skipped) = ai::clean_ranking(questions);
                if skipped {
                    send_error(&tx, ErrorCode::BadQuestion);
                }
                debug!(session, count = questions.len(), "new ranking");
                state.hub.set_ranking(session, questions);
            }
            Ok(AiMessage::Speech {
                question_id,
                format,
                audio,
            }) => match ai::decode_speech(&format, &audio) {
                Some(audio) => {
                    debug!(session, question_id, bytes = audio.len(), "speech ready");
                    state.hub.add_speech(session, question_id, Speech { format, audio });
                }
                None => send_error(&tx, ErrorCode::BadSpeech),
            },
            Err(err) => {
                debug!(session, %err, "bad AI message");
                send_error(&tx, ErrorCode::BadMessage);
            }
        }
    }

    state.hub.detach_ai(session);
    info!(session, "AI host disconnected");
    drop(tx);
    finish(writer).await;
}

/// Writes a frame to the recording, if there is one. A disk error stops the
/// recording but never the stream.
async fn record(recorder: &mut Option<Recorder>, kind: FrameKind, payload: &[u8]) {
    let Some(rec) = recorder else { return };
    let result = match kind {
        FrameKind::Video => rec.video(payload).await,
        FrameKind::Audio => rec.audio(payload).await,
        FrameKind::Speech => Ok(()),
    };
    if let Err(err) = result {
        warn!(%err, path = %rec.base().display(), "recording failed; stopping it");
        *recorder = None;
    }
}

fn send_error(tx: &mpsc::Sender<Message>, code: ErrorCode) {
    let _ = tx.try_send(Message::Text(code.to_text()));
}

/// Reads a socket, hiding pings and pongs, and gives up on a peer that goes quiet.
struct Inbound {
    stream: SplitStream<WebSocket>,
    idle: Duration,
}

impl Inbound {
    fn new(stream: SplitStream<WebSocket>, idle: Duration) -> Self {
        Self { stream, idle }
    }

    /// The next text or binary message; `None` once the peer closes, errors, or times out.
    async fn next(&mut self) -> Option<Message> {
        loop {
            match timeout(self.idle, self.stream.next()).await {
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
                Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => return None,
                Ok(Some(Ok(message))) => return Some(message),
            }
        }
    }
}

fn ping_timer(every: Duration) -> tokio::time::Interval {
    interval_at(Instant::now() + every, every)
}

/// Sends queued messages and pings until every sender is gone or a close goes out.
async fn write_loop(mut sink: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>, ping_every: Duration) {
    let mut ping = ping_timer(ping_every);
    loop {
        let message = tokio::select! {
            message = rx.recv() => match message {
                Some(message) => message,
                None => break,
            },
            _ = ping.tick() => Message::Ping(Bytes::new()),
        };
        let closing = matches!(message, Message::Close(_));
        if sink.send(message).await.is_err() || closing {
            return;
        }
    }
    let _ = sink.close().await;
}

async fn viewer_write_loop(
    mut sink: SplitSink<WebSocket, Message>,
    snapshot: Vec<ViewerEvent>,
    mut events: broadcast::Receiver<ViewerEvent>,
    mut direct: mpsc::Receiver<Message>,
    ping_every: Duration,
) {
    let mut gate = KeyframeGate::default();
    for event in snapshot {
        if let Some(message) = gate.filter(event)
            && sink.send(message).await.is_err()
        {
            return;
        }
    }
    let mut ping = ping_timer(ping_every);
    loop {
        let message = tokio::select! {
            event = events.recv() => match event {
                Ok(event) => match gate.filter(event) {
                    Some(message) => message,
                    None => continue,
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "viewer fell behind; waiting for the next keyframe");
                    gate.lagged();
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            message = direct.recv() => match message {
                Some(message) => message,
                None => break,
            },
            _ = ping.tick() => Message::Ping(Bytes::new()),
        };
        if sink.send(message).await.is_err() {
            return;
        }
    }
    let _ = sink.close().await;
}

/// Holds back video after a reset or a gap until a keyframe arrives, since the
/// decoder can't do anything with the frames in between.
#[derive(Default)]
struct KeyframeGate {
    open: bool,
}

impl KeyframeGate {
    fn filter(&mut self, event: ViewerEvent) -> Option<Message> {
        match event {
            ViewerEvent::StreamReset(text) => {
                self.open = false;
                Some(Message::Text(text))
            }
            ViewerEvent::Video { frame, keyframe } => {
                self.open |= keyframe;
                self.open.then_some(Message::Binary(frame))
            }
            ViewerEvent::Audio(frame) => Some(Message::Binary(frame)),
            ViewerEvent::Text(text) => Some(Message::Text(text)),
        }
    }

    fn lagged(&mut self) {
        self.open = false;
    }
}

/// Lets the writer flush what's queued, but not hang on a dead connection.
async fn finish(mut writer: JoinHandle<()>) {
    if timeout(Duration::from_secs(1), &mut writer).await.is_err() {
        writer.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(keyframe: bool) -> ViewerEvent {
        ViewerEvent::Video {
            frame: Bytes::from_static(b"frame"),
            keyframe,
        }
    }

    #[test]
    fn gate_waits_for_a_keyframe() {
        let mut gate = KeyframeGate::default();
        assert!(gate.filter(video(false)).is_none());
        assert!(gate.filter(video(true)).is_some());
        assert!(gate.filter(video(false)).is_some());
    }

    #[test]
    fn gate_closes_after_lag_and_reset() {
        let mut gate = KeyframeGate::default();
        gate.filter(video(true));
        gate.lagged();
        assert!(gate.filter(video(false)).is_none());
        assert!(gate.filter(ViewerEvent::Audio(Bytes::from_static(b"a"))).is_some());
        gate.filter(video(true));
        assert!(gate.filter(ViewerEvent::StreamReset("{}".into())).is_some());
        assert!(gate.filter(video(false)).is_none());
    }
}
