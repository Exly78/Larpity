mod common;

use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

use common::*;
use larpity_server::Config;
use serde_json::json;
use tokio_tungstenite::tungstenite::{self, Message};

fn no_rate_limit() -> Config {
    Config {
        chat_interval: Duration::ZERO,
        ..Config::default()
    }
}

async fn viewer(addr: std::net::SocketAddr) -> Ws {
    let mut ws = connect(addr, "/ws/viewer").await;
    expect_type(&mut ws, "offline").await;
    ws
}

async fn chatter(addr: std::net::SocketAddr, nick: &str) -> Ws {
    let mut ws = viewer(addr).await;
    send_json(&mut ws, json!({ "type": "join", "nick": nick })).await;
    expect_type(&mut ws, "joined").await;
    ws
}

/// Sends a chat message and waits for the server to echo it back.
async fn say(ws: &mut Ws, text: &str) {
    send_json(ws, json!({ "type": "chat", "text": text })).await;
    loop {
        let message = recv_json(ws).await;
        assert_ne!(message["type"], "error", "{message}");
        if message["type"] == "chat" && message["text"] == text {
            return;
        }
    }
}

#[tokio::test]
async fn ai_gets_every_chat_message() {
    let addr = start(Config::default()).await;
    let mut ai = connect(addr, "/ws/ai").await;
    let mut ana = chatter(addr, "Ana").await;
    say(&mut ana, "Ce ai studiat?").await;

    let chat = expect_type(&mut ai, "chat").await;
    assert_eq!(chat["nick"], "Ana");
    assert_eq!(chat["text"], "Ce ai studiat?");
    assert_eq!(chat["id"], 1);
}

#[tokio::test]
async fn ai_catches_up_on_recent_chat_when_it_connects() {
    let addr = start(no_rate_limit()).await;
    let mut ana = chatter(addr, "Ana").await;
    for text in ["one", "two", "three"] {
        say(&mut ana, text).await;
    }
    let mut ai = connect(addr, "/ws/ai").await;
    for text in ["one", "two", "three"] {
        assert_eq!(expect_type(&mut ai, "chat").await["text"], text);
    }
}

#[tokio::test]
async fn ranking_reaches_viewers_as_questions() {
    let addr = start(Config::default()).await;
    let mut ai = connect(addr, "/ws/ai").await;
    let mut early = viewer(addr).await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "q7", "text": "Cât durează o operație?", "count": 4, "chat_ids": [1, 2, 3, 4] },
            { "id": "q2", "text": "Ce ai studiat?" },
        ]}),
    )
    .await;

    let expected = json!([
        { "id": "q7", "text": "Cât durează o operație?", "count": 4 },
        { "id": "q2", "text": "Ce ai studiat?", "count": 1 },
    ]);
    assert_eq!(expect_type(&mut early, "questions").await["questions"], expected);
    // Late viewers get the current list when they join.
    let mut late = viewer(addr).await;
    assert_eq!(expect_type(&mut late, "questions").await["questions"], expected);
}

#[tokio::test]
async fn bad_questions_are_skipped_and_reported() {
    let addr = start(Config::default()).await;
    let mut ai = connect(addr, "/ws/ai").await;
    let mut watcher = viewer(addr).await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "", "text": "no id" },
            { "id": "q1", "text": "kept" },
        ]}),
    )
    .await;
    assert_eq!(expect_type(&mut ai, "error").await["code"], "bad_question");
    let questions = expect_type(&mut watcher, "questions").await;
    assert_eq!(
        questions["questions"],
        json!([{ "id": "q1", "text": "kept", "count": 1 }])
    );
}

#[tokio::test]
async fn bad_speech_and_bad_messages_are_reported() {
    let addr = start(Config::default()).await;
    let mut ai = connect(addr, "/ws/ai").await;
    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q1", "format": "mp3", "audio": "%%%" }),
    )
    .await;
    assert_eq!(expect_type(&mut ai, "error").await["code"], "bad_speech");
    send_json(&mut ai, json!({ "type": "ranking" })).await;
    assert_eq!(expect_type(&mut ai, "error").await["code"], "bad_message");
}

#[tokio::test]
async fn new_ai_host_replaces_the_old_one() {
    let addr = start(Config::default()).await;
    let mut old = connect(addr, "/ws/ai").await;
    let mut new = connect(addr, "/ws/ai").await;
    match recv(&mut old).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 4000),
        other => panic!("expected a close frame, got {other:?}"),
    }
    let mut ana = chatter(addr, "Ana").await;
    say(&mut ana, "hello").await;
    assert_eq!(expect_type(&mut new, "chat").await["text"], "hello");
}

#[tokio::test]
async fn ranking_survives_the_ai_host_disconnecting() {
    let addr = start(Config::default()).await;
    let mut ai = connect(addr, "/ws/ai").await;
    let mut watcher = viewer(addr).await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [{ "id": "q1", "text": "kept?" }] }),
    )
    .await;
    expect_type(&mut watcher, "questions").await;
    drop(ai);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut late = viewer(addr).await;
    assert_eq!(expect_type(&mut late, "questions").await["questions"][0]["id"], "q1");
}

#[tokio::test]
async fn ai_needs_the_token_when_one_is_set() {
    let addr = start(Config {
        token: Some("s3cret".into()),
        ..Config::default()
    })
    .await;
    match tokio_tungstenite::connect_async(format!("ws://{addr}/ws/ai?token=nope")).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
        other => panic!("expected 401, got {other:?}"),
    }
    connect(addr, "/ws/ai?token=s3cret").await;
}

/// Kills the child process when the test ends, pass or fail.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn stub_ai_merges_repeated_questions() {
    let addr = start(Config {
        token: Some("s3cret".into()),
        ..no_rate_limit()
    })
    .await;
    let _stub = Running(
        Command::new(env!("CARGO_BIN_EXE_stub-ai"))
            .args(["--url", &format!("ws://{addr}/ws/ai"), "--token", "s3cret"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut ana = chatter(addr, "Ana").await;
    let mut bob = chatter(addr, "Bob").await;
    say(&mut ana, "Ce ai studiat?").await;
    say(&mut bob, "ce ai studiat ??").await;
    say(&mut bob, "no question mark here").await;
    say(&mut ana, "Câți ani ai?").await;

    // Whether the stub connects before or after the chat, it ends up with the same ranking.
    let mut watcher = viewer(addr).await;
    for _ in 0..50 {
        let message = recv_json(&mut watcher).await;
        if message["type"] == "questions" && message["questions"].as_array().unwrap().len() == 2 {
            let questions = &message["questions"];
            if questions[0]["count"] == 2 {
                assert_eq!(questions[0]["text"], "Ce ai studiat?");
                assert_eq!(questions[1]["text"], "Câți ani ai?");
                assert_eq!(questions[1]["count"], 1);
                return;
            }
        }
    }
    panic!("never saw the merged ranking");
}
