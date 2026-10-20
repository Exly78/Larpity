mod common;

use std::{
    net::SocketAddr,
    process::{Child, Command, Stdio},
    time::Duration,
};

use common::*;
use larpity_server::{
    Config,
    protocol::{FrameHeader, FrameKind, HEADER_LEN},
};
use serde_json::{Value, json};

const AUDIO: &[u8] = b"RIFF fake wav bytes";
const AUDIO_BASE64: &str = "UklGRiBmYWtlIHdhdiBieXRlcw==";

fn no_debounce() -> Config {
    Config {
        button_debounce: Duration::ZERO,
        ..Config::default()
    }
}

/// A phone, an AI host that has ranked two questions with speech for the first, and a viewer.
async fn ready(addr: SocketAddr) -> (Ws, Ws, Ws) {
    let mut phone = connect(addr, "/ws/phone").await;
    let mut ai = connect(addr, "/ws/ai").await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "q1", "text": "Ce ai studiat?", "count": 3 },
            { "id": "q2", "text": "Câți ani ai?", "count": 1 },
        ]}),
    )
    .await;
    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q1", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    expect_type(&mut viewer, "questions").await;
    // The speech was sent after the ranking on the same socket, but make sure it is in.
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Nothing reaches the phone until the button is pressed.
    assert_quiet(&mut phone).await;
    (phone, ai, viewer)
}

async fn press(phone: &mut Ws) {
    send_json(phone, json!({ "type": "button" })).await;
}

fn ids(questions: &Value) -> Vec<&str> {
    questions["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn button_sends_speech_to_the_phone_and_tells_everyone() {
    let addr = start(Config::default()).await;
    let (mut phone, mut ai, mut viewer) = ready(addr).await;
    press(&mut phone).await;

    let picked = expect_type(&mut phone, "question_picked").await;
    assert_eq!(
        picked,
        json!({ "type": "question_picked", "id": "q1", "text": "Ce ai studiat?", "audio": true, "format": "wav" })
    );
    let frame = recv_binary(&mut phone).await;
    assert_eq!(FrameHeader::parse(&frame).unwrap().kind, FrameKind::Speech);
    assert_eq!(&frame[HEADER_LEN..], AUDIO);

    let for_others = json!({ "type": "question_picked", "id": "q1", "text": "Ce ai studiat?" });
    assert_eq!(expect_type(&mut viewer, "question_picked").await, for_others);
    assert_eq!(ids(&expect_type(&mut viewer, "questions").await), ["q2"]);
    assert_eq!(expect_type(&mut ai, "question_picked").await, for_others);
}

#[tokio::test]
async fn question_without_speech_yet_comes_as_text_only() {
    let addr = start(no_debounce()).await;
    let (mut phone, _ai, _viewer) = ready(addr).await;
    press(&mut phone).await;
    expect_type(&mut phone, "question_picked").await;
    recv_binary(&mut phone).await;

    press(&mut phone).await;
    let picked = expect_type(&mut phone, "question_picked").await;
    assert_eq!(picked["id"], "q2");
    assert_eq!(picked["audio"], false);
    assert!(picked.get("format").is_none());
    assert_quiet(&mut phone).await;
}

#[tokio::test]
async fn empty_queue_gets_an_error() {
    let addr = start(Config::default()).await;
    let mut phone = connect(addr, "/ws/phone").await;
    press(&mut phone).await;
    assert_eq!(expect_type(&mut phone, "error").await["code"], "no_question");
}

#[tokio::test]
async fn double_press_picks_only_one_question() {
    let addr = start(Config::default()).await;
    let (mut phone, _ai, mut viewer) = ready(addr).await;
    press(&mut phone).await;
    press(&mut phone).await;
    assert_eq!(expect_type(&mut phone, "question_picked").await["id"], "q1");
    recv_binary(&mut phone).await;
    assert_quiet(&mut phone).await;
    expect_type(&mut viewer, "question_picked").await;
    assert_eq!(ids(&expect_type(&mut viewer, "questions").await), ["q2"]);
    assert_quiet(&mut viewer).await;
}

#[tokio::test]
async fn picked_questions_are_never_listed_again() {
    let addr = start(Config::default()).await;
    let (mut phone, mut ai, mut viewer) = ready(addr).await;
    press(&mut phone).await;
    expect_type(&mut viewer, "question_picked").await;
    expect_type(&mut viewer, "questions").await;

    // The AI host hasn't caught up yet and sends q1 again.
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [
            { "id": "q1", "text": "Ce ai studiat?" }, { "id": "q3", "text": "Unde lucrezi?" },
        ]}),
    )
    .await;
    assert_eq!(ids(&expect_type(&mut viewer, "questions").await), ["q3"]);
}

#[tokio::test]
async fn late_viewers_see_the_question_being_answered() {
    let addr = start(Config::default()).await;
    let (mut phone, _ai, mut viewer) = ready(addr).await;
    press(&mut phone).await;
    expect_type(&mut viewer, "question_picked").await;

    let mut late = connect(addr, "/ws/viewer").await;
    expect_type(&mut late, "offline").await;
    assert_eq!(ids(&expect_type(&mut late, "questions").await), ["q2"]);
    assert_eq!(expect_type(&mut late, "question_picked").await["id"], "q1");
}

#[tokio::test]
async fn reconnecting_ai_host_hears_what_was_answered() {
    let addr = start(Config::default()).await;
    let (mut phone, _ai, mut viewer) = ready(addr).await;
    press(&mut phone).await;
    expect_type(&mut viewer, "question_picked").await;

    let mut new_ai = connect(addr, "/ws/ai").await;
    let picked = expect_type(&mut new_ai, "question_picked").await;
    assert_eq!(picked["id"], "q1");
    assert_eq!(picked["text"], "Ce ai studiat?");
}

#[tokio::test]
async fn only_the_current_phone_can_press() {
    let addr = start(Config::default()).await;
    let (mut old_phone, _ai, mut viewer) = ready(addr).await;
    let _new_phone = connect(addr, "/ws/phone").await;
    press(&mut old_phone).await;
    assert_quiet(&mut viewer).await;
}

/// Press until q1 (with speech) and q2 (without) are both picked; returns once the
/// phone has shown q2 as text only.
async fn press_through_q2(phone: &mut Ws, ai: &mut Ws, viewer: &mut Ws) {
    press(phone).await;
    expect_type(phone, "question_picked").await;
    recv_binary(phone).await;
    expect_type(viewer, "question_picked").await;
    expect_type(viewer, "questions").await;
    expect_type(ai, "question_picked").await;
    press(phone).await;
    let picked = expect_type(phone, "question_picked").await;
    assert_eq!(picked["id"], "q2");
    assert_eq!(picked["audio"], false);
    expect_type(viewer, "question_picked").await;
    expect_type(viewer, "questions").await;
    expect_type(ai, "question_picked").await;
}

#[tokio::test]
async fn speech_that_arrives_just_after_the_press_still_reaches_the_phone() {
    let addr = start(no_debounce()).await;
    let (mut phone, mut ai, mut viewer) = ready(addr).await;
    press_through_q2(&mut phone, &mut ai, &mut viewer).await;

    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q2", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    let again = expect_type(&mut phone, "question_picked").await;
    assert_eq!(
        again,
        json!({ "type": "question_picked", "id": "q2", "text": "Câți ani ai?", "audio": true, "format": "wav" })
    );
    let frame = recv_binary(&mut phone).await;
    assert_eq!(FrameHeader::parse(&frame).unwrap().kind, FrameKind::Speech);
    assert_eq!(&frame[HEADER_LEN..], AUDIO);
    // Only the phone hears about it, and only once.
    assert_quiet(&mut viewer).await;
    assert_quiet(&mut ai).await;
    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q2", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    assert_quiet(&mut phone).await;
}

#[tokio::test]
async fn speech_that_arrives_too_late_is_dropped() {
    let addr = start(Config {
        late_speech_window: Duration::ZERO,
        ..no_debounce()
    })
    .await;
    let (mut phone, mut ai, mut viewer) = ready(addr).await;
    press_through_q2(&mut phone, &mut ai, &mut viewer).await;

    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q2", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    assert_quiet(&mut phone).await;
}

#[tokio::test]
async fn speech_for_an_older_pick_is_dropped() {
    let addr = start(no_debounce()).await;
    let (mut phone, mut ai, mut viewer) = ready(addr).await;
    press_through_q2(&mut phone, &mut ai, &mut viewer).await;
    // A newer question gets picked, also without speech.
    send_json(
        &mut ai,
        json!({ "type": "ranking", "questions": [{ "id": "q3", "text": "Unde lucrezi?" }] }),
    )
    .await;
    expect_type(&mut viewer, "questions").await;
    press(&mut phone).await;
    assert_eq!(expect_type(&mut phone, "question_picked").await["id"], "q3");
    expect_type(&mut ai, "question_picked").await;

    // q2's speech is for a question the professional has moved past.
    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q2", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    assert_quiet(&mut phone).await;
    // q3's still counts.
    send_json(
        &mut ai,
        json!({ "type": "speech", "question_id": "q3", "format": "wav", "audio": AUDIO_BASE64 }),
    )
    .await;
    assert_eq!(expect_type(&mut phone, "question_picked").await["id"], "q3");
    recv_binary(&mut phone).await;
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn whole_loop_with_the_stub_ai() {
    let addr = start(Config {
        chat_interval: Duration::ZERO,
        ..Config::default()
    })
    .await;
    let _stub = Running(
        Command::new(env!("CARGO_BIN_EXE_stub-ai"))
            .args(["--url", &format!("ws://{addr}/ws/ai")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut phone = connect(addr, "/ws/phone").await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    send_json(&mut viewer, json!({ "type": "join", "nick": "Ana" })).await;
    expect_type(&mut viewer, "joined").await;
    send_json(&mut viewer, json!({ "type": "chat", "text": "Ce ai studiat?" })).await;
    expect_type(&mut viewer, "chat").await;
    // The stub ranks the question, then sends its speech.
    let questions = expect_type(&mut viewer, "questions").await;
    assert_eq!(questions["questions"][0]["text"], "Ce ai studiat?");
    tokio::time::sleep(Duration::from_millis(200)).await;

    press(&mut phone).await;
    let picked = expect_type(&mut phone, "question_picked").await;
    assert_eq!(picked["text"], "Ce ai studiat?");
    assert_eq!(picked["audio"], true);
    assert_eq!(picked["format"], "wav");
    let speech = recv_binary(&mut phone).await;
    assert_eq!(&speech[HEADER_LEN..HEADER_LEN + 4], b"RIFF");
    assert_eq!(
        expect_type(&mut viewer, "question_picked").await["text"],
        "Ce ai studiat?"
    );
}
