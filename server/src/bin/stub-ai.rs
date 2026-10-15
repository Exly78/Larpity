//! A stand-in AI host for testing without an LLM or TTS.
//!
//! Every chat message with a question mark becomes a question; repeats (same
//! words, ignoring case and punctuation) are merged with a count, and the most
//! asked come first. Each new question gets a short beep as its "speech".
//!
//!     cargo run --bin stub-ai -- --url ws://127.0.0.1:8080/ws/ai [--token SECRET]
//!
//! The real AI host (issues #28-#31) speaks the same protocol: see the AI
//! section of PROTOCOL.md.

use std::{
    env,
    process::exit,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const USAGE: &str = "usage: stub-ai [--url ws://HOST/ws/ai] [--token SECRET]

  --url    the server's AI endpoint (default ws://127.0.0.1:8080/ws/ai)
  --token  the server's LARPITY_TOKEN (default: the LARPITY_TOKEN variable)";

#[tokio::main]
async fn main() {
    let url = parse_args();
    loop {
        match run(&url).await {
            Ok(Stop::Replaced) => {
                eprintln!("another AI host took over; exiting");
                return;
            }
            Ok(Stop::Closed) => eprintln!("the server closed the connection"),
            Err(err) => eprintln!("connection failed: {err}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        eprintln!("reconnecting to {url}");
    }
}

fn parse_args() -> String {
    let mut url = "ws://127.0.0.1:8080/ws/ai".to_owned();
    let mut token = env::var("LARPITY_TOKEN").ok().filter(|t| !t.is_empty());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{arg} needs a value\n\n{USAGE}");
                exit(2)
            })
        };
        match arg.as_str() {
            "--url" => url = value(),
            "--token" => token = Some(value()),
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            _ => {
                eprintln!("unknown argument {arg}\n\n{USAGE}");
                exit(2)
            }
        }
    }
    match token {
        Some(token) => format!("{url}{}token={token}", if url.contains('?') { '&' } else { '?' }),
        None => url,
    }
}

enum Stop {
    Closed,
    Replaced,
}

async fn run(url: &str) -> Result<Stop, tokio_tungstenite::tungstenite::Error> {
    let (mut ws, _) = connect_async(url).await?;
    eprintln!("connected");
    // The server replays recent chat on connect, so start from nothing each time.
    let mut ranker = Ranker::new();
    while let Some(message) = ws.next().await {
        let text = match message? {
            Message::Text(text) => text,
            Message::Close(Some(frame)) if u16::from(frame.code) == 4000 => return Ok(Stop::Replaced),
            _ => continue,
        };
        let Ok(message) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let replies = match message["type"].as_str() {
            Some("chat") => ranker.chat(
                message["id"].as_u64().unwrap_or_default(),
                message["text"].as_str().unwrap_or_default(),
            ),
            Some("question_picked") => ranker.picked(
                message["id"].as_str().unwrap_or_default(),
                message["text"].as_str().unwrap_or_default(),
            ),
            Some("error") => {
                eprintln!("server says: {}", message["message"]);
                vec![]
            }
            _ => vec![],
        };
        for reply in replies {
            ws.send(Message::text(reply.to_string())).await?;
        }
    }
    Ok(Stop::Closed)
}

struct Ranker {
    /// Ids must never repeat, even across restarts, so they start with the connect time.
    prefix: u64,
    next_id: u32,
    questions: Vec<Entry>,
    /// Questions already answered, so a replayed chat doesn't bring them back.
    answered: Vec<String>,
}

struct Entry {
    id: String,
    key: String,
    text: String,
    count: u32,
    chat_ids: Vec<u64>,
}

impl Ranker {
    fn new() -> Self {
        let prefix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            prefix,
            next_id: 0,
            questions: Vec::new(),
            answered: Vec::new(),
        }
    }

    /// Returns the messages to send back.
    fn chat(&mut self, chat_id: u64, text: &str) -> Vec<Value> {
        let key = normalize(text);
        if !text.contains('?') || key.is_empty() || self.answered.contains(&key) {
            return vec![];
        }
        let mut speech = None;
        match self.questions.iter_mut().find(|q| q.key == key) {
            Some(question) => {
                question.count += 1;
                question.chat_ids.push(chat_id);
            }
            None => {
                self.next_id += 1;
                let id = format!("{}-{}", self.prefix, self.next_id);
                println!("new question {id}: {text}");
                speech = Some(json!({
                    "type": "speech", "question_id": id, "format": "wav", "audio": STANDARD.encode(beep()),
                }));
                self.questions.push(Entry {
                    id,
                    key,
                    text: text.trim().to_owned(),
                    count: 1,
                    chat_ids: vec![chat_id],
                });
            }
        }
        // Most asked first; the sort is stable, so ties stay in the order first asked.
        self.questions.sort_by_key(|q| std::cmp::Reverse(q.count));
        // Ranking first, then speech, like the real AI host (issue #31).
        [Some(self.ranking()), speech].into_iter().flatten().collect()
    }

    fn picked(&mut self, id: &str, text: &str) -> Vec<Value> {
        println!("picked {id}: {text}");
        let key = normalize(text);
        self.questions.retain(|q| q.id != id && q.key != key);
        self.answered.push(key);
        vec![self.ranking()]
    }

    fn ranking(&self) -> Value {
        let questions: Vec<Value> = self
            .questions
            .iter()
            .take(20)
            .map(|q| json!({ "id": q.id, "text": q.text, "count": q.count, "chat_ids": q.chat_ids }))
            .collect();
        json!({ "type": "ranking", "questions": questions })
    }
}

/// Lowercase words without punctuation, so "Ce faci?" and "ce faci ??" match.
fn normalize(text: &str) -> String {
    let letters: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .flat_map(char::to_lowercase)
        .collect();
    letters.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 0.4 s of 880 Hz fading out, as a 16 kHz mono WAV file.
fn beep() -> Vec<u8> {
    const RATE: u32 = 16_000;
    const SAMPLES: u32 = RATE * 2 / 5;
    let mut wav = Vec::with_capacity(44 + SAMPLES as usize * 2);
    let data_len = SAMPLES * 2;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes per sample
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..SAMPLES {
        let t = i as f32 / RATE as f32;
        let fade = 1.0 - i as f32 / SAMPLES as f32;
        let sample = (t * 880.0 * std::f32::consts::TAU).sin() * fade * 0.3 * f32::from(i16::MAX);
        wav.extend_from_slice(&(sample as i16).to_le_bytes());
    }
    wav
}
