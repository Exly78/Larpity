//! Shared state: who the phone is, what the stream looks like, and the fan-out to viewers.
//!
//! Everything sits behind one mutex so a viewer that joins gets a snapshot and a
//! subscription with nothing missed or repeated between them. The lock is only
//! ever held for a few field updates, never across an await.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes};
use bytes::Bytes;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use crate::{
    chat::{HISTORY, Nicknames},
    protocol::{ChatMessage, ServerMessage, StreamConfig},
};

/// How many events a viewer may fall behind before it skips ahead (~8 s of stream).
const VIEWER_BACKLOG: usize = 512;
/// Stop caching for late joiners if the phone goes this long without a keyframe (10 s at 15 fps).
const MAX_GOP_FRAMES: usize = 150;

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
    next_session: u64,
    phone: Option<Slot>,
    /// The `config` message, while the phone is streaming.
    stream: Option<Utf8Bytes>,
    /// Video frames since the last keyframe, so late joiners can start decoding at once.
    gop: Vec<Bytes>,
    next_chat_id: u64,
    history: VecDeque<ChatMessage>,
    nicknames: Nicknames,
}

/// The connection currently acting as the phone. Dropping it drops the sender,
/// which ends that connection's writer.
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

    /// What a new viewer needs to catch up, plus the live feed from that point on.
    pub fn join_viewer(&self) -> (Vec<ViewerEvent>, broadcast::Receiver<ViewerEvent>) {
        let inner = self.lock();
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
        (snapshot, self.viewers.subscribe())
    }

    /// Reserves a nickname (adding a number if it is taken) and frees the viewer's old one.
    pub fn claim_nick(&self, wanted: &str, previous: Option<&str>) -> String {
        let mut inner = self.lock();
        if let Some(previous) = previous {
            inner.nicknames.release(previous);
        }
        inner.nicknames.claim(wanted)
    }

    pub fn release_nick(&self, nick: &str) {
        self.lock().nicknames.release(nick);
    }

    /// Sends a chat message to every viewer and keeps it for late joiners.
    pub fn post_chat(&self, nick: &str, text: String) {
        let mut inner = self.lock();
        inner.next_chat_id += 1;
        let message = ChatMessage {
            id: inner.next_chat_id,
            nick: nick.to_owned(),
            text,
            ts: unix_millis(),
        };
        self.broadcast(ViewerEvent::Text(ServerMessage::Chat(&message).to_text()));
        if inner.history.len() == HISTORY {
            inner.history.pop_front();
        }
        inner.history.push_back(message);
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
