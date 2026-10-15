mod common;

use std::time::Duration;

use common::*;
use larpity_server::Config;
use serde_json::json;

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

async fn join(ws: &mut Ws, nick: &str) -> String {
    send_json(ws, json!({ "type": "join", "nick": nick })).await;
    let joined = expect_type(ws, "joined").await;
    joined["nick"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn chat_reaches_every_viewer_including_the_sender() {
    let addr = start(Config::default()).await;
    let mut ana = viewer(addr).await;
    let mut lurker = viewer(addr).await;
    assert_eq!(join(&mut ana, "Ana").await, "Ana");

    send_json(&mut ana, json!({ "type": "chat", "text": "Ce ai studiat?" })).await;
    for ws in [&mut ana, &mut lurker] {
        let chat = expect_type(ws, "chat").await;
        assert_eq!(chat["nick"], "Ana");
        assert_eq!(chat["text"], "Ce ai studiat?");
        assert_eq!(chat["id"], 1);
        assert!(chat["ts"].as_u64().unwrap() > 1_700_000_000_000);
    }
}

#[tokio::test]
async fn chatting_needs_a_nickname_first() {
    let addr = start(Config::default()).await;
    let mut ws = viewer(addr).await;
    send_json(&mut ws, json!({ "type": "chat", "text": "hello" })).await;
    assert_eq!(expect_type(&mut ws, "error").await["code"], "join_required");
}

#[tokio::test]
async fn taken_nicknames_get_a_number_and_are_freed_on_change() {
    let addr = start(Config::default()).await;
    let mut first = viewer(addr).await;
    let mut second = viewer(addr).await;
    assert_eq!(join(&mut first, "Ana").await, "Ana");
    assert_eq!(join(&mut second, "ana").await, "ana2");

    assert_eq!(join(&mut first, "Bob").await, "Bob");
    let mut third = viewer(addr).await;
    assert_eq!(join(&mut third, "Ana").await, "Ana");
}

#[tokio::test]
async fn nickname_is_freed_when_the_viewer_leaves() {
    let addr = start(Config::default()).await;
    let mut first = viewer(addr).await;
    join(&mut first, "Ana").await;
    drop(first);

    // The server frees the name once it notices the socket is gone.
    for _ in 0..50 {
        let mut next = viewer(addr).await;
        if join(&mut next, "Ana").await == "Ana" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nickname was never freed");
}

#[tokio::test]
async fn bad_nicknames_and_messages_are_refused() {
    let addr = start(no_rate_limit()).await;
    let mut ws = viewer(addr).await;
    for nick in ["   ", &"x".repeat(25)] {
        send_json(&mut ws, json!({ "type": "join", "nick": nick })).await;
        assert_eq!(expect_type(&mut ws, "error").await["code"], "bad_nick");
    }
    join(&mut ws, "Ana").await;
    for text in ["", " \n ", &"x".repeat(201)] {
        send_json(&mut ws, json!({ "type": "chat", "text": text })).await;
        assert_eq!(expect_type(&mut ws, "error").await["code"], "bad_text");
    }
    send_json(&mut ws, json!({ "type": "shout" })).await;
    assert_eq!(expect_type(&mut ws, "error").await["code"], "bad_message");
}

#[tokio::test]
async fn messages_are_cleaned_before_sending() {
    let addr = start(Config::default()).await;
    let mut ws = viewer(addr).await;
    assert_eq!(join(&mut ws, "  Ana\u{202E} ").await, "Ana");
    send_json(&mut ws, json!({ "type": "chat", "text": "  two\n\nlines\u{7} " })).await;
    assert_eq!(expect_type(&mut ws, "chat").await["text"], "two lines");
}

#[tokio::test]
async fn one_message_per_second() {
    let addr = start(Config::default()).await;
    let mut ws = viewer(addr).await;
    join(&mut ws, "Ana").await;
    send_json(&mut ws, json!({ "type": "chat", "text": "first" })).await;
    expect_type(&mut ws, "chat").await;
    send_json(&mut ws, json!({ "type": "chat", "text": "too soon" })).await;
    assert_eq!(expect_type(&mut ws, "error").await["code"], "rate_limited");
}

#[tokio::test]
async fn late_viewers_get_the_last_50_messages() {
    let addr = start(no_rate_limit()).await;
    let mut ws = viewer(addr).await;
    join(&mut ws, "Ana").await;
    for i in 1..=55 {
        send_json(&mut ws, json!({ "type": "chat", "text": format!("message {i}") })).await;
        expect_type(&mut ws, "chat").await;
    }

    let mut late = viewer(addr).await;
    for i in 6..=55 {
        let chat = expect_type(&mut late, "chat").await;
        assert_eq!(chat["id"], i);
        assert_eq!(chat["text"], format!("message {i}"));
    }
    assert_quiet(&mut late).await;
}

#[tokio::test]
async fn chat_history_comes_after_the_stream_snapshot() {
    let addr = start(Config::default()).await;
    let (_phone, mut early) = live_stream(addr).await;
    join(&mut early, "Ana").await;
    send_json(&mut early, json!({ "type": "chat", "text": "hi" })).await;
    expect_type(&mut early, "chat").await;

    let mut late = connect(addr, "/ws/viewer").await;
    expect_type(&mut late, "config").await;
    expect_type(&mut late, "chat").await;
}
