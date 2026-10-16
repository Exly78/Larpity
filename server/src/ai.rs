//! Checking what the AI host sends before it reaches viewers or the phone.

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;

use crate::{
    chat,
    protocol::{Question, RankedQuestion},
};

pub const MAX_QUESTIONS: usize = 20;
pub const MAX_ID_CHARS: usize = 64;
pub const MAX_QUESTION_CHARS: usize = 300;
pub const MAX_SPEECH_BYTES: usize = 2 << 20;

/// Keeps the first 20 valid questions, cleaning their text. The flag is true if
/// any were skipped for a missing or too-long id or text, or a repeated id.
pub fn clean_ranking(ranked: Vec<RankedQuestion>) -> (Vec<Question>, bool) {
    let mut questions: Vec<Question> = Vec::with_capacity(ranked.len().min(MAX_QUESTIONS));
    let mut skipped = false;
    for question in ranked {
        if questions.len() == MAX_QUESTIONS {
            break;
        }
        let id_ok = (1..=MAX_ID_CHARS).contains(&question.id.chars().count());
        let text = chat::clean(&question.text, MAX_QUESTION_CHARS);
        match text {
            Some(text) if id_ok && !questions.iter().any(|q| q.id == question.id) => questions.push(Question {
                id: question.id,
                text,
                count: question.count.max(1),
            }),
            _ => skipped = true,
        }
    }
    (questions, skipped)
}

/// Decodes a speech clip; `None` if the format name is odd, the base64 is bad,
/// or the clip is empty or over 2 MB.
pub fn decode_speech(format: &str, audio: &str) -> Option<Bytes> {
    let format_ok = (1..=16).contains(&format.len()) && format.bytes().all(|b| b.is_ascii_alphanumeric());
    // Base64 is 4 characters per 3 bytes; refuse oversized clips before decoding.
    if !format_ok || audio.len() / 4 * 3 > MAX_SPEECH_BYTES + 3 {
        return None;
    }
    let bytes = STANDARD.decode(audio).ok()?;
    (!bytes.is_empty() && bytes.len() <= MAX_SPEECH_BYTES).then(|| bytes.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(id: &str, text: &str, count: u32) -> RankedQuestion {
        RankedQuestion {
            id: id.into(),
            text: text.into(),
            count,
            chat_ids: vec![],
        }
    }

    #[test]
    fn ranking_keeps_valid_questions_in_order() {
        let (questions, skipped) = clean_ranking(vec![
            ranked("q2", "  Cât  câștigi? ", 3),
            ranked("q1", "Ce ai studiat?", 0),
        ]);
        assert!(!skipped);
        assert_eq!(questions[0].text, "Cât câștigi?");
        assert_eq!(questions[1].id, "q1");
        assert_eq!(questions[1].count, 1);
    }

    #[test]
    fn ranking_skips_bad_and_repeated_questions() {
        let (questions, skipped) = clean_ranking(vec![
            ranked("", "no id", 1),
            ranked("q1", "   ", 1),
            ranked(&"x".repeat(65), "id too long", 1),
            ranked("q2", &"y".repeat(301), 1),
            ranked("q3", "fine", 1),
            ranked("q3", "same id again", 1),
        ]);
        assert!(skipped);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "q3");
        assert_eq!(questions[0].text, "fine");
    }

    #[test]
    fn ranking_is_capped_at_20() {
        let many = (0..30).map(|i| ranked(&format!("q{i}"), "?", 1)).collect();
        let (questions, skipped) = clean_ranking(many);
        assert_eq!(questions.len(), 20);
        assert!(!skipped);
    }

    #[test]
    fn speech_is_decoded_and_checked() {
        assert_eq!(decode_speech("mp3", "aGVsbG8=").unwrap(), Bytes::from_static(b"hello"));
        assert_eq!(decode_speech("mp3", "not base64!"), None);
        assert_eq!(decode_speech("mp3", ""), None);
        assert_eq!(decode_speech("", "aGVsbG8="), None);
        assert_eq!(decode_speech("../x", "aGVsbG8="), None);
        let too_big = STANDARD.encode(vec![0u8; MAX_SPEECH_BYTES + 1]);
        assert_eq!(decode_speech("wav", &too_big), None);
        let just_fits = STANDARD.encode(vec![0u8; MAX_SPEECH_BYTES]);
        assert!(decode_speech("wav", &just_fits).is_some());
    }
}
