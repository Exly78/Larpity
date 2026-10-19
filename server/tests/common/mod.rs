//! Helpers for driving a real server over real WebSockets.
#![allow(dead_code)]

use std::{net::SocketAddr, time::Duration};

use futures_util::{SinkExt, StreamExt};
use larpity_server::{
    Config,
    protocol::{FrameKind, encode_frame},
};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAIT: Duration = Duration::from_secs(3);

pub async fn start(config: Config) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, larpity_server::app(config)).await.unwrap() });
    addr
}

pub async fn connect(addr: SocketAddr, path: &str) -> Ws {
    connect_async(format!("ws://{addr}{path}")).await.unwrap().0
}

pub async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::text(value.to_string())).await.unwrap();
}

/// The next text or binary message, skipping pings. Panics if the socket closes or stays quiet.
pub async fn recv(ws: &mut Ws) -> Message {
    loop {
        let message = timeout(WAIT, ws.next())
            .await
            .expect("timed out waiting for a message")
            .expect("socket closed")
            .expect("socket error");
        match message {
            Message::Ping(_) | Message::Pong(_) => continue,
            other => return other,
        }
    }
}

pub async fn recv_json(ws: &mut Ws) -> Value {
    match recv(ws).await {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected JSON, got {other:?}"),
    }
}

pub async fn recv_binary(ws: &mut Ws) -> Vec<u8> {
    match recv(ws).await {
        Message::Binary(bytes) => bytes.to_vec(),
        other => panic!("expected a binary frame, got {other:?}"),
    }
}

/// Reads JSON messages until one has the given type, failing on anything else in between.
pub async fn expect_type(ws: &mut Ws, kind: &str) -> Value {
    let value = recv_json(ws).await;
    assert_eq!(value["type"], kind, "unexpected message: {value}");
    value
}

/// Asserts nothing but pings arrives for a moment.
pub async fn assert_quiet(ws: &mut Ws) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_) => return,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(other) => panic!("expected silence, got {other:?}"),
        }
    }
}

/// True once the server has closed the socket.
pub async fn closed(ws: &mut Ws) -> bool {
    loop {
        match timeout(WAIT, ws.next()).await {
            Err(_) => return false,
            Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => return true,
            Ok(Some(Ok(_))) => continue,
        }
    }
}

pub fn config_message() -> Value {
    serde_json::json!({
        "type": "config",
        "video": { "codec": "avc1.42E01F", "width": 1280, "height": 720, "framerate": 15 },
        "audio": { "codec": "pcm16", "sample_rate": 16000, "channels": 1 },
    })
}

pub fn video(keyframe: bool, ts: u64, payload: &[u8]) -> Message {
    Message::binary(encode_frame(FrameKind::Video, keyframe, ts, payload).to_vec())
}

pub fn audio(ts: u64, payload: &[u8]) -> Message {
    Message::binary(encode_frame(FrameKind::Audio, false, ts, payload).to_vec())
}

/// A connected phone that has sent its config, and a viewer that has seen it.
pub async fn live_stream(addr: SocketAddr) -> (Ws, Ws) {
    let mut viewer = connect(addr, "/ws/viewer").await;
    expect_type(&mut viewer, "offline").await;
    let mut phone = connect(addr, "/ws/phone").await;
    send_json(&mut phone, config_message()).await;
    expect_type(&mut viewer, "config").await;
    (phone, viewer)
}

/// A plain HTTP response: status, headers (names lowercased) and body.
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("body is JSON")
    }
}

/// `GET path` over a fresh TCP connection, with no HTTP client in the way.
pub async fn http_get(addr: SocketAddr, path: &str) -> HttpResponse {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut raw = Vec::new();
    timeout(WAIT, stream.read_to_end(&mut raw)).await.unwrap().unwrap();
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no end of headers");
    let head = std::str::from_utf8(&raw[..end]).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    HttpResponse {
        status,
        headers,
        body: raw[end + 4..].to_vec(),
    }
}

/// A fresh empty directory under the system temp dir.
pub fn scratch_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("larpity-{name}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
