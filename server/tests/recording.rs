mod common;

use std::{fs, path::PathBuf, time::SystemTime};

use common::*;
use futures_util::SinkExt;
use larpity_server::Config;

fn scratch_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("larpity-recording-test-{nanos}"))
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
