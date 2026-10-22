//! Shared state: who the phone is, what the stream looks like, and the fan-out to viewers.
//!
//! Everything sits behind one mutex so a viewer that joins gets a snapshot and a
//! subscription with nothing missed or repeated between them. The lock is only
//! ever held for a few field updates, never across an await.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes};
use bytes::Bytes;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use crate::recording::{EventLog, LogFile};
use crate::{
    chat::{HISTORY, Nicknames, fresh_key},
    protocol::{ChatMessage, FrameKind, Question, ServerMessage, StreamConfig, encode_frame},
};

/// How many events a viewer may fall behind before it skips ahead (~8 s of stream).
const VIEWER_BACKLOG: usize = 512;
/// Stop caching for late joiners if the phone goes this long without a keyframe (10 s at 15 fps).
const MAX_GOP_FRAMES: usize = 150;
/// Speech clips kept at once; well above the 20 questions a ranking can hold.
const MAX_SPEECH_CLIPS: usize = 64;

/// What every viewer receives.
#[derive(Debug, Clone)]
pub enum ViewerEvent {
    /// `config` or `offline`: the decoder starts over and must wait for a keyframe.
    StreamReset(Utf8Bytes),
    Video {
        frame: Bytes,
        keyframe: bool,
    },
    Audio(Bytes),
    /// Any other message, e.g. chat.
    Text(Utf8Bytes),
}

pub struct Hub {
    inner: Mutex<Inner>,
    viewers: broadcast::Sender<ViewerEvent>,
}

#[derive(Default)]
struct Inner {
    /// The chat and questions log, when recording is on.
    log: Option<EventLog>,
    next_session: u64,
    phone: Option<Slot>,
    /// The `config` message, while the phone is streaming.
    stream: Option<Utf8Bytes>,
    /// Video frames since the last keyframe, so late joiners can start decoding at once.
    gop: Vec<Bytes>,
    next_chat_id: u64,
    history: VecDeque<ChatMessage>,
    nicknames: Nicknames,
    /// Each joined viewer's nickname and key, by session.
    claims: HashMap<u64, Claim>,
    /// Which session a key belongs to.
    keys: HashMap<String, u64>,
    ai: Option<Slot>,
    /// The AI host's latest ranking, best first.
    questions: Vec<Question>,
    /// Speech clips by question id.
    speech: HashMap<String, Speech>,
    /// Every question id ever picked, so the AI host can't list one again.
    answered: HashSet<String>,
    /// The most recent picks, replayed to an AI host that reconnects.
    picked: VecDeque<Question>,
    last_pick: Option<Instant>,
    /// The last pick, if it went to the phone without speech: its id and when.
    awaiting_speech: Option<(String, Instant)>,
}

/// What a button press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Picked,
    /// Nothing in the queue.
    Empty,
    /// Too soon after the previous press; treated as the same press.
    Ignored,
}

/// What became of a speech clip from the AI host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechFate {
    /// Kept for the button.
    Stored,
    /// Its question was just picked without it, so it went straight to the phone.
    SentLate,
    /// Its question was picked too long ago, or before a newer one; nobody needs it.
    Dropped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub format: String,
    pub audio: Bytes,
}

/// The connection currently acting as the phone or the AI host.
struct Slot {
    session: u64,
    tx: mpsc::Sender<Message>,
}

/// Close code telling a phone or AI host that a newer connection took its place,
/// so it should not reconnect (two phones would otherwise keep kicking each other off).
pub const CLOSE_REPLACED: u16 = 4000;

impl Slot {
    fn replaced(self, reason: &'static str) {
        let _ = self.tx.try_send(Message::Close(Some(CloseFrame {
            code: CLOSE_REPLACED,
            reason: reason.into(),
        })));
    }
}

/// A viewer's nickname, and the key that gets it back after a reconnect.
struct Claim {
    nick: String,
    key: String,
}

impl Inner {
    /// Frees a viewer's nickname and key; returns the claim, if there was one.
    fn release_claim(&mut self, session: u64) -> Option<Claim> {
        let claim = self.claims.remove(&session)?;
        self.nicknames.release(&claim.nick);
        self.keys.remove(&claim.key);
        Some(claim)
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::default(),
            viewers: broadcast::channel(VIEWER_BACKLOG).0,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn broadcast(&self, event: ViewerEvent) {
        // An error only means nobody is watching.
        let _ = self.viewers.send(event);
    }

    pub fn viewer_count(&self) -> usize {
        self.viewers.receiver_count()
    }

    /// Keeps a log of the chat and the questions in `dir`, next to the recordings.
    pub fn log_to(&self, dir: std::path::PathBuf) {
        self.lock().log = Some(EventLog::spawn(dir));
    }

    /// Makes this connection the phone, replacing any earlier one. Returns its session id.
    pub fn attach_phone(&self, tx: mpsc::Sender<Message>) -> u64 {
        let mut inner = self.lock();
        inner.next_session += 1;
        let session = inner.next_session;
        if let Some(old) = inner.phone.replace(Slot { session, tx }) {
            warn!("a new phone connected; dropping the old one");
            old.replaced("another phone took over");
        }
        self.end_stream(&mut inner);
        session
    }

    pub fn is_phone(&self, session: u64) -> bool {
        self.lock().phone.as_ref().is_some_and(|p| p.session == session)
    }

    pub fn detach_phone(&self, session: u64) {
        let mut inner = self.lock();
        if inner.phone.as_ref().is_some_and(|p| p.session == session) {
            inner.phone = None;
            self.end_stream(&mut inner);
        }
    }

    fn end_stream(&self, inner: &mut Inner) {
        inner.gop.clear();
        if inner.stream.take().is_some() {
            self.broadcast(ViewerEvent::StreamReset(ServerMessage::Offline.to_text()));
        }
    }

    pub fn set_config(&self, session: u64, config: &StreamConfig) {
        let mut inner = self.lock();
        if !inner.phone.as_ref().is_some_and(|p| p.session == session) {
            return;
        }
        let text = ServerMessage::Config(config).to_text();
        inner.stream = Some(text.clone());
        inner.gop.clear();
        self.broadcast(ViewerEvent::StreamReset(text));
    }

    pub fn publish_video(&self, session: u64, frame: Bytes, keyframe: bool) {
        let mut inner = self.lock();
        if !inner.phone.as_ref().is_some_and(|p| p.session == session) || inner.stream.is_none() {
            return;
        }
        if keyframe {
            inner.gop.clear();
        }
        // Only cache once there is a keyframe to start from, and give up if the GOP runs long.
        if keyframe || !inner.gop.is_empty() {
            if inner.gop.len() < MAX_GOP_FRAMES {
                inner.gop.push(frame.clone());
            } else {
                inner.gop.clear();
            }
        }
        self.broadcast(ViewerEvent::Video { frame, keyframe });
    }

    pub fn publish_audio(&self, session: u64, frame: Bytes) {
        let inner = self.lock();
        if inner.phone.as_ref().is_some_and(|p| p.session == session) && inner.stream.is_some() {
            self.broadcast(ViewerEvent::Audio(frame));
        }
    }

    /// What a new viewer needs to catch up, plus the live feed from that point on,
    /// and the viewer's session id.
    pub fn join_viewer(&self) -> (u64, Vec<ViewerEvent>, broadcast::Receiver<ViewerEvent>) {
        let mut inner = self.lock();
        inner.next_session += 1;
        let session = inner.next_session;
        let mut snapshot = Vec::with_capacity(1 + inner.gop.len());
        match &inner.stream {
            Some(config) => {
                snapshot.push(ViewerEvent::StreamReset(config.clone()));
                snapshot.extend(inner.gop.iter().enumerate().map(|(i, frame)| ViewerEvent::Video {
                    frame: frame.clone(),
                    keyframe: i == 0,
                }));
            }
            None => snapshot.push(ViewerEvent::StreamReset(ServerMessage::Offline.to_text())),
        }
        snapshot.extend(
            inner
                .history
                .iter()
                .map(|message| ViewerEvent::Text(ServerMessage::Chat(message).to_text())),
        );
        if !inner.questions.is_empty() {
            snapshot.push(ViewerEvent::Text(
                ServerMessage::Questions {
                    questions: &inner.questions,
                }
                .to_text(),
            ));
        }
        if let Some(current) = inner.picked.back() {
            snapshot.push(ViewerEvent::Text(picked_message(current)));
        }
        (session, snapshot, self.viewers.subscribe())
    }

    /// Gives the viewer a nickname (with a number added if it is taken) and the
    /// key that keeps it. The viewer's own earlier nickname is freed first (a
    /// change of name), and so is the one held by `key`'s connection, if that is
    /// another session (a reconnect taking its name back).
    pub fn claim_nick(&self, session: u64, wanted: &str, key: Option<&str>) -> (String, String) {
        let mut inner = self.lock();
        if let Some(old) = key.and_then(|key| inner.keys.get(key).copied())
            && old != session
        {
            inner.release_claim(old);
        }
        let key = inner.release_claim(session).map_or_else(fresh_key, |claim| claim.key);
        let nick = inner.nicknames.claim(wanted);
        inner.keys.insert(key.clone(), session);
        inner.claims.insert(
            session,
            Claim {
                nick: nick.clone(),
                key: key.clone(),
            },
        );
        (nick, key)
    }

    /// The viewer left: its nickname is free again and its key forgotten.
    pub fn leave_viewer(&self, session: u64) {
        self.lock().release_claim(session);
    }

    /// Sends a chat message from this viewer to everyone and keeps it for late
    /// joiners. `false` if the viewer has no nickname (never joined, or another
    /// connection took its name back with the key).
    pub fn post_chat(&self, session: u64, text: String) -> bool {
        let mut inner = self.lock();
        let Some(nick) = inner.claims.get(&session).map(|claim| claim.nick.clone()) else {
            return false;
        };
        inner.next_chat_id += 1;
        let message = ChatMessage {
            id: inner.next_chat_id,
            nick,
            text,
            ts: unix_millis(),
        };
        let text = ServerMessage::Chat(&message).to_text();
        if let Some(ai) = &inner.ai {
            let _ = ai.tx.try_send(Message::Text(text.clone()));
        }
        log_event(&inner, LogFile::Chat, &text);
        self.broadcast(ViewerEvent::Text(text));
        if inner.history.len() == HISTORY {
            inner.history.pop_front();
        }
        inner.history.push_back(message);
        true
    }

    /// Makes this connection the AI host, replacing any earlier one, and queues
    /// the recent chat so it can catch up. Returns its session id.
    pub fn attach_ai(&self, tx: mpsc::Sender<Message>) -> u64 {
        let mut inner = self.lock();
        inner.next_session += 1;
        let session = inner.next_session;
        for message in &inner.history {
            let _ = tx.try_send(Message::Text(ServerMessage::Chat(message).to_text()));
        }
        // Then what was already answered, so the AI host doesn't rank it again.
        for question in &inner.picked {
            let _ = tx.try_send(Message::Text(picked_message(question)));
        }
        if let Some(old) = inner.ai.replace(Slot { session, tx }) {
            warn!("a new AI host connected; dropping the old one");
            old.replaced("another AI host took over");
        }
        session
    }

    pub fn is_ai(&self, session: u64) -> bool {
        self.lock().ai.as_ref().is_some_and(|ai| ai.session == session)
    }

    /// Forgets the connection but keeps its last ranking and speech, so the
    /// button still works while the AI host restarts.
    pub fn detach_ai(&self, session: u64) {
        let mut inner = self.lock();
        if inner.ai.as_ref().is_some_and(|ai| ai.session == session) {
            inner.ai = None;
        }
    }

    pub fn set_ranking(&self, session: u64, questions: Vec<Question>) {
        let mut inner = self.lock();
        if !inner.ai.as_ref().is_some_and(|ai| ai.session == session) {
            return;
        }
        inner.questions = questions;
        let Inner {
            questions, answered, ..
        } = &mut *inner;
        questions.retain(|q| !answered.contains(&q.id));
        let message = ServerMessage::Questions {
            questions: &inner.questions,
        };
        log_event(&inner, LogFile::Questions, &stamped(&message));
        self.broadcast(ViewerEvent::Text(message.to_text()));
    }

    /// Keeps a speech clip for the button, or sends it on to the phone if its
    /// question was picked without it less than `late_window` ago.
    pub fn add_speech(&self, session: u64, question_id: String, speech: Speech, late_window: Duration) -> SpeechFate {
        let mut inner = self.lock();
        if !inner.ai.as_ref().is_some_and(|ai| ai.session == session) {
            return SpeechFate::Dropped;
        }
        if inner.awaiting_speech.as_ref().is_some_and(|(id, _)| *id == question_id) {
            let (_, pressed_at) = inner.awaiting_speech.take().expect("checked above");
            let question = inner.picked.back().filter(|q| q.id == question_id);
            let phone = inner.phone.as_ref().map(|p| p.tx.clone());
            return match (question, phone) {
                (Some(question), Some(phone_tx)) if pressed_at.elapsed() <= late_window => {
                    send_pick_to_phone(&phone_tx, question, Some(&speech));
                    SpeechFate::SentLate
                }
                _ => SpeechFate::Dropped,
            };
        }
        if inner.answered.contains(&question_id) {
            return SpeechFate::Dropped;
        }
        if inner.speech.len() >= MAX_SPEECH_CLIPS && !inner.speech.contains_key(&question_id) {
            let Inner { speech, questions, .. } = &mut *inner;
            speech.retain(|id, _| questions.iter().any(|q| &q.id == id));
        }
        inner.speech.insert(question_id, speech);
        SpeechFate::Stored
    }

    /// The button: takes the top question, sends its speech to the phone, and
    /// tells viewers and the AI host it was picked.
    pub fn pick_question(&self, session: u64, debounce: Duration) -> Pick {
        let mut inner = self.lock();
        let Some(phone) = inner.phone.as_ref().filter(|p| p.session == session) else {
            return Pick::Ignored;
        };
        let phone_tx = phone.tx.clone();
        if inner.last_pick.is_some_and(|at| at.elapsed() < debounce) {
            return Pick::Ignored;
        }
        if inner.questions.is_empty() {
            return Pick::Empty;
        }
        let now = Instant::now();
        inner.last_pick = Some(now);
        let question = inner.questions.remove(0);
        let speech = inner.speech.remove(&question.id);
        send_pick_to_phone(&phone_tx, &question, speech.as_ref());
        // If the speech is still being made, it can follow within the late window.
        inner.awaiting_speech = speech.is_none().then(|| (question.id.clone(), now));

        let picked = picked_message(&question);
        if let Some(ai) = &inner.ai {
            let _ = ai.tx.try_send(Message::Text(picked.clone()));
        }
        self.broadcast(ViewerEvent::Text(picked));
        self.broadcast(ViewerEvent::Text(
            ServerMessage::Questions {
                questions: &inner.questions,
            }
            .to_text(),
        ));

        inner.answered.insert(question.id.clone());
        log_event(
            &inner,
            LogFile::Questions,
            &stamped(&ServerMessage::QuestionPicked {
                id: &question.id,
                text: &question.text,
                audio: None,
                format: None,
            }),
        );
        log_event(
            &inner,
            LogFile::Questions,
            &stamped(&ServerMessage::Questions {
                questions: &inner.questions,
            }),
        );
        if inner.picked.len() == HISTORY {
            inner.picked.pop_front();
        }
        inner.picked.push_back(question);
        Pick::Picked
    }
}

/// `question_picked` as the phone gets it, with the speech frame when there is one.
fn send_pick_to_phone(phone_tx: &mpsc::Sender<Message>, question: &Question, speech: Option<&Speech>) {
    let _ = phone_tx.try_send(Message::Text(
        ServerMessage::QuestionPicked {
            id: &question.id,
            text: &question.text,
            audio: Some(speech.is_some()),
            format: speech.map(|s| s.format.as_str()),
        }
        .to_text(),
    ));
    if let Some(speech) = speech {
        let frame = encode_frame(FrameKind::Speech, false, unix_micros(), &speech.audio);
        let _ = phone_tx.try_send(Message::Binary(frame));
    }
}

/// `question_picked` as viewers and the AI host get it.
fn picked_message(question: &Question) -> Utf8Bytes {
    ServerMessage::QuestionPicked {
        id: &question.id,
        text: &question.text,
        audio: None,
        format: None,
    }
    .to_text()
}

fn unix_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Appends a line to the chat or questions log, if recording is on.
fn log_event(inner: &Inner, file: LogFile, line: &str) {
    if let Some(log) = &inner.log {
        log.log(file, line.to_owned());
    }
}

/// The message as JSON with `"ts"` (Unix ms) in front, for the questions log.
fn stamped(message: &ServerMessage<'_>) -> String {
    #[derive(serde::Serialize)]
    struct Stamped<'a> {
        ts: u64,
        #[serde(flatten)]
        message: &'a ServerMessage<'a>,
    }
    serde_json::to_string(&Stamped {
        ts: unix_millis(),
        message,
    })
    .expect("server messages always serialize")
}
