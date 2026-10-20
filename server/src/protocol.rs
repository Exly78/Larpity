//! The wire format shared by the phone, viewers and AI host. PROTOCOL.md at the
//! repo root is the reference; keep the two in step.

use axum::extract::ws::Utf8Bytes;
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};

/// Every binary frame starts with: kind (1 byte), flags (1 byte), timestamp in µs (8 bytes, big-endian).
pub const HEADER_LEN: usize = 10;
const FLAG_KEYFRAME: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Video = 1,
    Audio = 2,
    Speech = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub kind: FrameKind,
    pub keyframe: bool,
    pub timestamp_us: u64,
}

impl FrameHeader {
    /// Reads the header of a binary frame; `None` if it is too short or of an unknown kind.
    pub fn parse(frame: &[u8]) -> Option<Self> {
        if frame.len() < HEADER_LEN {
            return None;
        }
        let kind = match frame[0] {
            1 => FrameKind::Video,
            2 => FrameKind::Audio,
            3 => FrameKind::Speech,
            _ => return None,
        };
        let timestamp_us = u64::from_be_bytes(frame[2..HEADER_LEN].try_into().unwrap());
        Some(Self {
            kind,
            keyframe: frame[1] & FLAG_KEYFRAME != 0,
            timestamp_us,
        })
    }
}

pub fn encode_frame(kind: FrameKind, keyframe: bool, timestamp_us: u64, payload: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(HEADER_LEN + payload.len());
    out.put_u8(kind as u8);
    out.put_u8(if keyframe { FLAG_KEYFRAME } else { 0 });
    out.put_u64(timestamp_us);
    out.put_slice(payload);
    out.freeze()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamConfig {
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: Option<AudioConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoConfig {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framerate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

/// Text messages the phone sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneMessage {
    Config(StreamConfig),
    /// The "next question" button.
    Button,
}

/// Text messages a viewer sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerMessage {
    Join {
        nick: String,
        /// From an earlier `joined`: takes the nickname back after a reconnect.
        #[serde(default)]
        key: Option<String>,
    },
    Chat {
        text: String,
    },
}

/// Text messages the AI host sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiMessage {
    /// The full ranked list, best first, replacing the previous one.
    Ranking { questions: Vec<RankedQuestion> },
    /// Speech for one question; `audio` is base64.
    Speech {
        question_id: String,
        format: String,
        audio: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct RankedQuestion {
    pub id: String,
    pub text: String,
    #[serde(default = "one")]
    pub count: u32,
    /// Which chat messages asked it. Accepted for the AI host's own bookkeeping; not forwarded.
    #[serde(default)]
    pub chat_ids: Vec<u64>,
}

fn one() -> u32 {
    1
}

/// A ranked question as viewers see it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Question {
    pub id: String,
    pub text: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMessage {
    pub id: u64,
    pub nick: String,
    pub text: String,
    /// Unix time in milliseconds.
    pub ts: u64,
}

/// Text messages the server sends.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage<'a> {
    Config(&'a StreamConfig),
    Offline,
    Joined {
        nick: &'a str,
        /// Sent back with the next `join` to keep the nickname across a reconnect.
        key: &'a str,
    },
    Chat(&'a ChatMessage),
    Questions {
        questions: &'a [Question],
    },
    /// To viewers and the AI host: just `id` and `text`. To the phone, also
    /// whether a speech frame follows and its format.
    QuestionPicked {
        id: &'a str,
        text: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        audio: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<&'a str>,
    },
    Error {
        code: &'a str,
        message: &'a str,
    },
}

impl ServerMessage<'_> {
    pub fn to_text(&self) -> Utf8Bytes {
        serde_json::to_string(self)
            .expect("server messages always serialize")
            .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    BadMessage,
    BadFrame,
    ConfigRequired,
    JoinRequired,
    BadNick,
    BadText,
    RateLimited,
    BadQuestion,
    BadSpeech,
    NoQuestion,
}

impl ErrorCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::BadMessage => "bad_message",
            Self::BadFrame => "bad_frame",
            Self::ConfigRequired => "config_required",
            Self::JoinRequired => "join_required",
            Self::BadNick => "bad_nick",
            Self::BadText => "bad_text",
            Self::RateLimited => "rate_limited",
            Self::BadQuestion => "bad_question",
            Self::BadSpeech => "bad_speech",
            Self::NoQuestion => "no_question",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::BadMessage => "Not valid JSON, or an unknown message type",
            Self::BadFrame => "Binary frame too short or of an unknown kind",
            Self::ConfigRequired => "Send a config message before any media",
            Self::JoinRequired => "Pick a nickname (join) before chatting",
            Self::BadNick => "Nicknames are 1 to 24 characters",
            Self::BadText => "Messages are 1 to 200 characters",
            Self::RateLimited => "One message per second, please",
            Self::BadQuestion => {
                "Some ranked questions had a missing or too-long id or text, or a repeated id, and were skipped"
            }
            Self::BadSpeech => "Speech must be valid base64, at most 2 MB, with a short alphanumeric format",
            Self::NoQuestion => "No questions in the queue yet",
        }
    }

    pub fn to_text(self) -> Utf8Bytes {
        ServerMessage::Error {
            code: self.code(),
            message: self.message(),
        }
        .to_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_round_trips() {
        let frame = encode_frame(FrameKind::Video, true, 1_234_567, b"abc");
        assert_eq!(frame.len(), HEADER_LEN + 3);
        let header = FrameHeader::parse(&frame).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                kind: FrameKind::Video,
                keyframe: true,
                timestamp_us: 1_234_567
            }
        );
        assert_eq!(&frame[HEADER_LEN..], b"abc");
    }

    #[test]
    fn frame_header_rejects_short_and_unknown_frames() {
        assert_eq!(FrameHeader::parse(&[1, 0, 0]), None);
        assert_eq!(
            FrameHeader::parse(&encode_frame(FrameKind::Audio, false, 0, b"")[..9]),
            None
        );
        let mut frame = encode_frame(FrameKind::Audio, false, 0, b"x").to_vec();
        frame[0] = 9;
        assert_eq!(FrameHeader::parse(&frame), None);
    }

    #[test]
    fn config_message_parses_and_serializes_with_type() {
        let json = r#"{"type":"config","video":{"codec":"avc1.42E01F","width":1280,"height":720,"framerate":15},
                       "audio":{"codec":"pcm16","sample_rate":16000,"channels":1},"extra":"ignored"}"#;
        let Ok(PhoneMessage::Config(config)) = serde_json::from_str(json) else {
            panic!("not a config message");
        };
        assert_eq!(config.video.width, 1280);
        assert_eq!(config.audio.as_ref().unwrap().sample_rate, 16000);

        let out: serde_json::Value = serde_json::from_str(&ServerMessage::Config(&config).to_text()).unwrap();
        assert_eq!(out["type"], "config");
        assert_eq!(out["video"]["codec"], "avc1.42E01F");
        assert_eq!(out["audio"]["channels"], 1);
    }

    #[test]
    fn button_parses() {
        assert!(matches!(
            serde_json::from_str(r#"{"type":"button"}"#),
            Ok(PhoneMessage::Button)
        ));
    }

    #[test]
    fn question_picked_leaves_out_phone_only_fields_for_others() {
        let for_viewers = ServerMessage::QuestionPicked {
            id: "q1",
            text: "Ce ai studiat?",
            audio: None,
            format: None,
        };
        assert_eq!(
            for_viewers.to_text().as_str(),
            r#"{"type":"question_picked","id":"q1","text":"Ce ai studiat?"}"#
        );
        let for_phone = ServerMessage::QuestionPicked {
            id: "q1",
            text: "Ce ai studiat?",
            audio: Some(true),
            format: Some("mp3"),
        };
        assert_eq!(
            for_phone.to_text().as_str(),
            r#"{"type":"question_picked","id":"q1","text":"Ce ai studiat?","audio":true,"format":"mp3"}"#
        );
    }

    #[test]
    fn config_without_audio_is_accepted() {
        let json = r#"{"type":"config","video":{"codec":"avc1.42E01F","width":1280,"height":720}}"#;
        let Ok(PhoneMessage::Config(config)) = serde_json::from_str(json) else {
            panic!("not a config message");
        };
        assert_eq!(config.audio, None);
        assert_eq!(config.video.framerate, None);
    }
}

#[cfg(test)]
mod viewer_tests {
    use super::*;

    #[test]
    fn join_with_and_without_a_key() {
        let Ok(ViewerMessage::Join { nick, key }) = serde_json::from_str(r#"{"type":"join","nick":"Ana"}"#) else {
            panic!("not a join");
        };
        assert_eq!((nick.as_str(), key), ("Ana", None));
        let Ok(ViewerMessage::Join { key, .. }) = serde_json::from_str(r#"{"type":"join","nick":"Ana","key":"abc"}"#)
        else {
            panic!("not a join");
        };
        assert_eq!(key.as_deref(), Some("abc"));
        assert_eq!(
            ServerMessage::Joined {
                nick: "Ana2",
                key: "abc"
            }
            .to_text()
            .as_str(),
            r#"{"type":"joined","nick":"Ana2","key":"abc"}"#
        );
    }
}
