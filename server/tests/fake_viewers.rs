mod common;

use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

use common::*;
use larpity_server::Config;
use serde_json::Value;

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/baseline-aud-96x64.h264");

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn fake_viewers_watch_the_fake_phone_and_report() {
    let addr = start(Config {
        token: Some("s3cret".into()),
        chat_interval: Duration::ZERO,
        ..Config::default()
    })
    .await;
    let _phone = Running(
        Command::new(env!("CARGO_BIN_EXE_fake-phone"))
            .args(["--clip", CLIP, "--fps", "30"])
            .args(["--url", &format!("ws://{addr}/ws/phone"), "--token", "s3cret"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    // Wait for the stream to be live before the viewers arrive.
    let mut viewer = connect(addr, "/ws/viewer").await;
    loop {
        let message = recv_json(&mut viewer).await;
        if message["type"] == "config" {
            break;
        }
    }

    let url = format!("ws://{addr}/ws/viewer");
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_fake-viewers"))
            .args([
                "--url",
                &url,
                "--count",
                "3",
                "--seconds",
                "2",
                "--chat-every",
                "0.5",
                "--json",
            ])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let summary: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| panic!("not JSON: {stdout}"));

    assert_eq!(summary["viewers"], 3);
    assert_eq!(summary["connected"], 3);
    assert_eq!(summary["seconds"], 2.0);
    assert!(summary["video_fps"].as_f64().unwrap() > 10.0, "{summary}");
    assert!(summary["kbit_s"].as_f64().unwrap() > 0.0, "{summary}");
    assert!(summary["first_keyframe_ms"]["median"].is_u64(), "{summary}");
    assert_eq!(summary["gaps"], 0);
    assert!(summary["chat_rtt_ms"]["median"].is_u64(), "{summary}");
    assert_eq!(summary["chat_lost"], 0);
    assert_eq!(summary["disconnects"], 0);
    assert_eq!(summary["connect_failures"], 0);
}

#[tokio::test]
async fn fake_viewers_fail_when_nobody_connects() {
    let output = tokio::task::spawn_blocking(|| {
        Command::new(env!("CARGO_BIN_EXE_fake-viewers"))
            .args([
                "--url",
                "ws://127.0.0.1:9/ws/viewer",
                "--count",
                "1",
                "--seconds",
                "0.2",
                "--json",
            ])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    let summary: Value = serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(summary["connected"], 0);
    assert!(summary["connect_failures"].as_u64().unwrap() >= 1);
}
