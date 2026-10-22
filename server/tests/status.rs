mod common;

use std::fs;

use common::*;
use larpity_server::Config;
use serde_json::{Value, json};

async fn status(addr: std::net::SocketAddr) -> Value {
    let response = http_get(addr, "/status").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("content-type"), Some("application/json"));
    response.json()
}

#[tokio::test]
async fn status_starts_empty_and_follows_the_session() {
    let dir = scratch_dir("status");
    let addr = start(Config {
        record_dir: Some(dir.clone()),
        chat_interval: std::time::Duration::ZERO,
        ..Config::default()
    })
    .await;

    let empty = status(addr).await;
    assert!(empty["uptime_s"].is_u64());
    assert_eq!(
        empty,
        json!({
            "uptime_s": empty["uptime_s"], "phone": false, "stream": null, "recording": null,
            "viewers": 0, "ai": false, "questions": 0, "picked": 0, "chat_messages": 0,
        })
    );

    let (mut phone, mut viewer) = live_stream(addr).await;
    let mut ai = connect(addr, "/ws/ai").await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "q1", "text": "Ce ai studiat?", "count": 2 },
            { "id": "q2", "text": "Unde lucrezi?" },
        ]}),
    )
    .await;
    expect_type(&mut viewer, "questions").await;
    send_json(&mut viewer, json!({ "type": "join", "nick": "Ana" })).await;
    expect_type(&mut viewer, "joined").await;
    send_json(&mut viewer, json!({ "type": "chat", "text": "Ce ai studiat?" })).await;
    expect_type(&mut viewer, "chat").await;
    send_json(&mut phone, json!({ "type": "button" })).await;
    expect_type(&mut phone, "question_picked").await;
    expect_type(&mut viewer, "question_picked").await;
    expect_type(&mut viewer, "questions").await;

    let live = status(addr).await;
    assert_eq!(live["phone"], true);
    assert_eq!(live["stream"]["video"]["codec"], "avc1.42E01F");
    assert_eq!(live["stream"]["audio"]["sample_rate"], 16000);
    let recording = live["recording"].as_str().expect("a recording path");
    assert!(recording.starts_with(dir.to_str().unwrap()), "{recording}");
    assert!(fs::metadata(format!("{recording}.h264")).is_ok());
    assert_eq!(live["viewers"], 1);
    assert_eq!(live["ai"], true);
    assert_eq!(live["questions"], 1);
    assert_eq!(live["picked"], 1);
    assert_eq!(live["chat_messages"], 1);

    drop(phone);
    expect_type(&mut viewer, "offline").await;
    let after = status(addr).await;
    assert_eq!(after["phone"], false);
    assert_eq!(after["stream"], Value::Null);
    assert_eq!(after["recording"], Value::Null);
    // The queue and the counts survive the phone leaving.
    assert_eq!(after["questions"], 1);
    assert_eq!(after["picked"], 1);

    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn status_without_recording_has_no_path() {
    let addr = start(Config::default()).await;
    let (_phone, _viewer) = live_stream(addr).await;
    let live = status(addr).await;
    assert_eq!(live["phone"], true);
    assert_eq!(live["recording"], Value::Null);
}
