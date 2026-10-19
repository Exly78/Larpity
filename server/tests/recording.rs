mod common;

use std::{fs, path::PathBuf, time::Duration};

use common::*;
use futures_util::SinkExt;
use larpity_server::Config;
use serde_json::{Value, json};

fn scratch_dir() -> PathBuf {
    common::scratch_dir("recording")
}

fn files_with_extension(dir: &PathBuf, ext: &str) -> Vec<PathBuf> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == ext))
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn stream_is_saved_as_h264_pcm_and_json() {
    let dir = scratch_dir();
    let addr = start(Config {
        record_dir: Some(dir.clone()),
        ..Config::default()
    })
    .await;
    let (mut phone, mut viewer) = live_stream(addr).await;
    for message in [
        video(true, 0, b"\x00\x00\x00\x01key"),
        audio(0, b"pcm-a"),
        video(false, 1, b"\x00\x00\x00\x01delta"),
        audio(1, b"pcm-b"),
    ] {
        phone.send(message).await.unwrap();
    }
    phone.close(None).await.unwrap();
    // The phone's frames are written before it is detached, so once viewers hear
    // it is offline the files are complete.
    for _ in 0..4 {
        recv_binary(&mut viewer).await;
    }
    expect_type(&mut viewer, "offline").await;

    let h264 = files_with_extension(&dir, "h264");
    assert_eq!(h264.len(), 1);
    assert_eq!(fs::read(&h264[0]).unwrap(), b"\x00\x00\x00\x01key\x00\x00\x00\x01delta");
    assert_eq!(fs::read(h264[0].with_extension("pcm")).unwrap(), b"pcm-apcm-b");
    let config: serde_json::Value = serde_json::from_slice(&fs::read(h264[0].with_extension("json")).unwrap()).unwrap();
    assert_eq!(config["video"]["codec"], "avc1.42E01F");

    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn each_config_starts_a_new_recording() {
    let dir = scratch_dir();
    let addr = start(Config {
        record_dir: Some(dir.clone()),
        ..Config::default()
    })
    .await;
    let (mut phone, mut viewer) = live_stream(addr).await;
    phone.send(video(true, 0, b"first")).await.unwrap();
    recv_binary(&mut viewer).await;
    send_json(&mut phone, config_message()).await;
    expect_type(&mut viewer, "config").await;
    phone.send(video(true, 0, b"second")).await.unwrap();
    recv_binary(&mut viewer).await;
    phone.close(None).await.unwrap();
    expect_type(&mut viewer, "offline").await;

    let mut contents: Vec<_> = files_with_extension(&dir, "h264")
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    contents.sort();
    assert_eq!(contents, [b"first".to_vec(), b"second".to_vec()]);

    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn nothing_is_saved_when_recording_is_off() {
    let addr = start(Config::default()).await;
    let (mut phone, mut viewer) = live_stream(addr).await;
    phone.send(video(true, 0, b"key")).await.unwrap();
    recv_binary(&mut viewer).await;
}

/// The lines of the one `<prefix>-*.jsonl` file in `dir`, once it has `count` of them.
async fn log_lines(dir: &PathBuf, prefix: &str, count: usize) -> Vec<Value> {
    for _ in 0..100 {
        let files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_str().unwrap();
                name.starts_with(prefix) && name.ends_with(".jsonl")
            })
            .collect();
        if let [file] = files.as_slice() {
            let lines: Vec<Value> = fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            if lines.len() >= count {
                return lines;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{prefix} log never got {count} lines");
}

#[tokio::test]
async fn chat_and_questions_are_logged_next_to_the_recording() {
    let dir = scratch_dir();
    let addr = start(Config {
        record_dir: Some(dir.clone()),
        ..Config::default()
    })
    .await;
    let (mut phone, mut viewer) = live_stream(addr).await;
    send_json(&mut viewer, json!({ "type": "join", "nick": "Ana" })).await;
    expect_type(&mut viewer, "joined").await;
    send_json(&mut viewer, json!({ "type": "chat", "text": "Ce ai studiat?" })).await;
    let chat = expect_type(&mut viewer, "chat").await;

    let mut ai = connect(addr, "/ws/ai").await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "q1", "text": "Ce ai studiat?", "count": 1 },
            { "id": "q2", "text": "Unde lucrezi?", "count": 1 },
        ]}),
    )
    .await;
    let listed = expect_type(&mut viewer, "questions").await;
    send_json(&mut phone, json!({ "type": "button" })).await;
    expect_type(&mut phone, "question_picked").await;
    let picked = expect_type(&mut viewer, "question_picked").await;
    let remaining = expect_type(&mut viewer, "questions").await;

    // The chat file holds the message exactly as the viewer got it.
    let now = chat["ts"].as_u64().unwrap();
    assert_eq!(log_lines(&dir, "chat-", 1).await, [chat]);

    // The questions file holds every list and every pick, each with a time.
    let lines = log_lines(&dir, "questions-", 3).await;
    for line in &lines {
        let ts = line["ts"].as_u64().expect("a ts");
        assert!(ts >= now && ts < now + 10_000, "{line}");
    }
    let without_ts = |line: &Value| {
        let mut line = line.clone();
        line.as_object_mut().unwrap().remove("ts");
        line
    };
    assert_eq!(without_ts(&lines[0]), listed);
    assert_eq!(without_ts(&lines[1]), picked);
    assert_eq!(without_ts(&lines[2]), remaining);

    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn logs_are_written_even_while_no_phone_is_streaming() {
    let dir = scratch_dir();
    let addr = start(Config {
        record_dir: Some(dir.clone()),
        ..Config::default()
    })
    .await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    send_json(&mut viewer, json!({ "type": "join", "nick": "Ana" })).await;
    expect_type(&mut viewer, "joined").await;
    send_json(&mut viewer, json!({ "type": "chat", "text": "E cineva?" })).await;
    expect_type(&mut viewer, "chat").await;

    let lines = log_lines(&dir, "chat-", 1).await;
    assert_eq!(lines[0]["text"], "E cineva?");
    assert_eq!(lines[0]["nick"], "Ana");
    assert!(files_with_extension(&dir, "h264").is_empty());

    fs::remove_dir_all(dir).unwrap();
}
