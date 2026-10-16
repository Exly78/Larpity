mod common;

use std::time::Duration;

use common::*;
use futures_util::SinkExt;
use larpity_server::{Config, protocol::HEADER_LEN};
use tokio_tungstenite::tungstenite::{self, Message};

#[tokio::test]
async fn frames_reach_every_viewer_in_order_and_unchanged() {
    let addr = start(Config::default()).await;
    let (mut phone, mut first) = live_stream(addr).await;
    let mut second = connect(addr, "/ws/viewer").await;
    expect_type(&mut second, "config").await;

    let sent = [
        video(true, 0, b"key"),
        audio(10, b"pcm"),
        video(false, 66_666, b"delta"),
    ];
    for message in &sent {
        phone.send(message.clone()).await.unwrap();
    }
    for viewer in [&mut first, &mut second] {
        for message in &sent {
            assert_eq!(recv_binary(viewer).await, message.clone().into_data().to_vec());
        }
    }
}

#[tokio::test]
async fn config_is_forwarded_to_viewers() {
    let addr = start(Config::default()).await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    let mut phone = connect(addr, "/ws/phone").await;
    send_json(&mut phone, config_message()).await;

    let config = expect_type(&mut viewer, "config").await;
    assert_eq!(config["video"]["codec"], "avc1.42E01F");
    assert_eq!(config["video"]["width"], 1280);
    assert_eq!(config["audio"]["sample_rate"], 16000);
}

#[tokio::test]
async fn late_viewer_starts_from_the_latest_keyframe() {
    let addr = start(Config::default()).await;
    let (mut phone, mut early) = live_stream(addr).await;
    for message in [
        video(true, 0, b"old key"),
        video(false, 1, b"old delta"),
        video(true, 2, b"new key"),
        video(false, 3, b"new delta"),
    ] {
        phone.send(message).await.unwrap();
    }
    // Once the early viewer has everything, the server has processed it all.
    for _ in 0..4 {
        recv_binary(&mut early).await;
    }

    let mut late = connect(addr, "/ws/viewer").await;
    expect_type(&mut late, "config").await;
    assert_eq!(&recv_binary(&mut late).await[HEADER_LEN..], b"new key");
    assert_eq!(&recv_binary(&mut late).await[HEADER_LEN..], b"new delta");

    phone.send(video(false, 4, b"live")).await.unwrap();
    assert_eq!(&recv_binary(&mut late).await[HEADER_LEN..], b"live");
}

#[tokio::test]
async fn viewer_that_joins_mid_gop_without_cache_waits_for_a_keyframe() {
    let addr = start(Config::default()).await;
    let (mut phone, _early) = live_stream(addr).await;
    // Delta frames with no keyframe yet: nothing is cached, and a new viewer can't use them.
    phone.send(video(false, 0, b"orphan")).await.unwrap();
    let mut late = connect(addr, "/ws/viewer").await;
    expect_type(&mut late, "config").await;
    phone.send(video(false, 1, b"orphan 2")).await.unwrap();
    phone.send(video(true, 2, b"key")).await.unwrap();
    assert_eq!(&recv_binary(&mut late).await[HEADER_LEN..], b"key");
}

#[tokio::test]
async fn media_before_config_is_dropped_with_one_error() {
    let addr = start(Config::default()).await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    let mut phone = connect(addr, "/ws/phone").await;

    phone.send(video(true, 0, b"too early")).await.unwrap();
    phone.send(video(false, 1, b"still too early")).await.unwrap();
    let error = expect_type(&mut phone, "error").await;
    assert_eq!(error["code"], "config_required");
    assert_quiet(&mut phone).await;
    assert_quiet(&mut viewer).await;
}

#[tokio::test]
async fn bad_frames_and_bad_json_get_errors() {
    let addr = start(Config::default()).await;
    let mut phone = connect(addr, "/ws/phone").await;

    phone.send(Message::binary(vec![1, 0, 0])).await.unwrap();
    assert_eq!(expect_type(&mut phone, "error").await["code"], "bad_frame");
    phone.send(Message::text("not json")).await.unwrap();
    assert_eq!(expect_type(&mut phone, "error").await["code"], "bad_message");
    phone.send(Message::text(r#"{"type":"dance"}"#)).await.unwrap();
    assert_eq!(expect_type(&mut phone, "error").await["code"], "bad_message");
}

#[tokio::test]
async fn viewers_hear_when_the_phone_leaves() {
    let addr = start(Config::default()).await;
    let (mut phone, mut viewer) = live_stream(addr).await;
    phone.close(None).await.unwrap();
    expect_type(&mut viewer, "offline").await;
}

#[tokio::test]
async fn new_phone_replaces_the_old_one() {
    let addr = start(Config::default()).await;
    let (mut old_phone, mut viewer) = live_stream(addr).await;

    let mut new_phone = connect(addr, "/ws/phone").await;
    expect_type(&mut viewer, "offline").await;
    match recv(&mut old_phone).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 4000),
        other => panic!("expected a close frame, got {other:?}"),
    }

    send_json(&mut new_phone, config_message()).await;
    expect_type(&mut viewer, "config").await;
    new_phone.send(video(true, 0, b"from the new phone")).await.unwrap();
    assert_eq!(&recv_binary(&mut viewer).await[HEADER_LEN..], b"from the new phone");
}

#[tokio::test]
async fn phone_needs_the_token_when_one_is_set() {
    let addr = start(Config {
        token: Some("s3cret".into()),
        ..Config::default()
    })
    .await;

    for path in ["/ws/phone", "/ws/phone?token=wrong"] {
        match tokio_tungstenite::connect_async(format!("ws://{addr}{path}")).await {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
            other => panic!("expected 401 for {path}, got {other:?}"),
        }
    }
    let mut phone = connect(addr, "/ws/phone?token=s3cret").await;
    send_json(&mut phone, config_message()).await;
    // Viewers never need a token.
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "config").await;
}

#[tokio::test]
async fn silent_phone_is_dropped() {
    let addr = start(Config {
        ping_every: Duration::from_millis(100),
        idle_timeout: Duration::from_millis(400),
        ..Config::default()
    })
    .await;
    // The phone socket is never polled again, so it never answers pings.
    let (_phone, mut viewer) = live_stream(addr).await;
    expect_type(&mut viewer, "offline").await;
}
