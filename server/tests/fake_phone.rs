mod common;

use std::process::{Child, Command, Stdio};

use common::*;
use larpity_server::{
    Config, annexb,
    protocol::{FrameHeader, FrameKind, HEADER_LEN},
};
use tokio_tungstenite::tungstenite::Message;

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/baseline-aud-96x64.h264");

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn fake_phone_streams_the_clip_with_a_tone() {
    let addr = start(Config {
        token: Some("s3cret".into()),
        ..Config::default()
    })
    .await;
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;

    let _phone = Running(
        Command::new(env!("CARGO_BIN_EXE_fake-phone"))
            .args(["--clip", CLIP, "--once", "--fps", "30"])
            .args(["--url", &format!("ws://{addr}/ws/phone"), "--token", "s3cret"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let config = expect_type(&mut viewer, "config").await;
    assert_eq!(config["video"]["codec"], "avc1.42C00A");
    assert_eq!(config["video"]["width"], 96);
    assert_eq!(config["video"]["height"], 64);
    assert_eq!(config["video"]["framerate"], 30.0);
    assert_eq!(config["audio"]["codec"], "pcm16");

    let mut video = Vec::new();
    let mut audio_frames = 0;
    loop {
        match recv(&mut viewer).await {
            Message::Binary(frame) => {
                let header = FrameHeader::parse(&frame).unwrap();
                match header.kind {
                    FrameKind::Video => {
                        video.push((frame[HEADER_LEN..].to_vec(), header.keyframe, header.timestamp_us))
                    }
                    FrameKind::Audio => {
                        assert_eq!(frame.len() - HEADER_LEN, 640, "20 ms of 16 kHz 16-bit mono");
                        audio_frames += 1;
                    }
                    FrameKind::Speech => panic!("viewers never get speech"),
                }
            }
            Message::Text(text) => {
                assert!(text.contains(r#""type":"offline""#), "unexpected {text}");
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    let clip = std::fs::read(CLIP).unwrap();
    let expected: Vec<_> = annexb::access_units(&clip)
        .iter()
        .map(|unit| (unit.data.to_vec(), unit.keyframe))
        .collect();
    let got: Vec<_> = video.iter().map(|(data, key, _)| (data.clone(), *key)).collect();
    assert_eq!(got, expected);
    let timestamps: Vec<_> = video.iter().map(|(_, _, ts)| *ts).collect();
    assert_eq!(timestamps, [0, 33_333, 66_666, 100_000, 133_333]);
    assert!(audio_frames > 0);
}
