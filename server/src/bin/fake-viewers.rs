//! Opens many viewer connections at once and reports what each one saw, so the
//! fan-out and the VPS bandwidth can be checked before real students arrive.
//!
//!     cargo run --release --bin fake-viewers -- --url wss://host/ws/viewer --count 25 --minutes 30
//!
//! Each viewer joins with a nickname, sends a chat message now and then, and
//! counts the frames it gets. At the end it prints a table and one line of JSON.

use std::{env, process::exit, time::Duration};

use futures_util::{SinkExt, StreamExt};
use larpity_server::protocol::{FrameHeader, FrameKind};
use serde_json::{Value, json};
use tokio::time::{Instant, MissedTickBehavior, interval, sleep, sleep_until};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const USAGE: &str = "usage: fake-viewers [options]

  --url URL        the server's viewer endpoint (default ws://127.0.0.1:8080/ws/viewer)
  --count N        viewers to open (default 10)
  --minutes M      how long to watch (default 1)
  --seconds S      same, in seconds, for short runs
  --chat-every S   seconds between chat messages per viewer (default 20; 0 = never)
  --json           print only the one-line JSON summary";

/// A video timestamp jump longer than this counts as a gap (a lag skip or a stall).
const GAP: Duration = Duration::from_millis(500);
const BACKOFF: [u64; 4] = [1, 2, 4, 10];

struct Args {
    url: String,
    count: usize,
    duration: Duration,
    chat_every: Option<Duration>,
    json_only: bool,
}

/// What one viewer saw.
#[derive(Default)]
struct Stats {
    /// Got as far as `joined` at least once.
    connected: bool,
    connect_failures: u32,
    /// The server or the network closed the socket before the end.
    disconnects: u32,
    video_frames: u64,
    keyframes: u64,
    audio_frames: u64,
    bytes: u64,
    /// From the socket opening to the first keyframe, on the first connection.
    first_keyframe_ms: Option<u64>,
    gaps: u32,
    chat_rtt_ms: Vec<u64>,
    /// Chat messages sent that never came back.
    chat_lost: u32,
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let deadline = Instant::now() + args.duration;
    if !args.json_only {
        eprintln!(
            "{} viewers on {} for {:.0} s",
            args.count,
            args.url,
            args.duration.as_secs_f64()
        );
    }
    let mut tasks = Vec::with_capacity(args.count);
    for i in 0..args.count {
        let url = args.url.clone();
        let chat_every = args.chat_every;
        tasks.push(tokio::spawn(async move {
            // Don't hit the server with every connection in the same millisecond.
            sleep(Duration::from_millis(20 * i as u64)).await;
            watch(&url, i + 1, chat_every, deadline).await
        }));
    }
    let mut all = Vec::with_capacity(args.count);
    for task in tasks {
        all.push(task.await.expect("a viewer task panicked"));
    }
    let summary = summarize(&all, args.duration);
    if !args.json_only {
        print_table(&all);
        eprintln!();
    }
    println!("{summary}");
    if all.iter().any(|s| !s.connected) {
        eprintln!("some viewers never connected");
        exit(1)
    }
}

fn parse_args() -> Args {
    let mut url = "ws://127.0.0.1:8080/ws/viewer".to_owned();
    let mut count = 10;
    let mut duration = Duration::from_secs(60);
    let mut chat_every = Some(Duration::from_secs(20));
    let mut json_only = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{arg} needs a value\n\n{USAGE}");
                exit(2)
            })
        };
        let number = |text: String, what: &str| -> f64 {
            text.parse().ok().filter(|n: &f64| *n >= 0.0).unwrap_or_else(|| {
                eprintln!("{what} needs a number\n\n{USAGE}");
                exit(2)
            })
        };
        match arg.as_str() {
            "--url" => url = value(),
            "--count" => count = number(value(), "--count") as usize,
            "--minutes" => duration = Duration::from_secs_f64(number(value(), "--minutes") * 60.0),
            "--seconds" => duration = Duration::from_secs_f64(number(value(), "--seconds")),
            "--chat-every" => {
                let seconds = number(value(), "--chat-every");
                chat_every = (seconds > 0.0).then(|| Duration::from_secs_f64(seconds));
            }
            "--json" => json_only = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            _ => {
                eprintln!("unknown argument {arg}\n\n{USAGE}");
                exit(2)
            }
        }
    }
    if count == 0 {
        eprintln!("--count must be at least 1");
        exit(2)
    }
    Args {
        url,
        count,
        duration,
        chat_every,
        json_only,
    }
}

/// One viewer, reconnecting like the real page does, until the deadline.
async fn watch(url: &str, number: usize, chat_every: Option<Duration>, deadline: Instant) -> Stats {
    let nick = format!("Viewer{number}");
    let mut stats = Stats::default();
    let mut key: Option<String> = None;
    let mut failures = 0usize;
    let mut first_connection = true;
    while Instant::now() < deadline {
        let ws = match connect_async(url).await {
            Ok((ws, _)) => ws,
            Err(_) => {
                stats.connect_failures += 1;
                let wait = Duration::from_secs(BACKOFF[failures.min(BACKOFF.len() - 1)]);
                failures += 1;
                sleep_until((Instant::now() + wait).min(deadline)).await;
                continue;
            }
        };
        failures = 0;
        let opened = Instant::now();
        let (mut sink, mut incoming) = ws.split();
        let mut join = json!({ "type": "join", "nick": nick });
        if let Some(key) = &key {
            join["key"] = json!(key);
        }
        if sink.send(Message::text(join.to_string())).await.is_err() {
            stats.disconnects += 1;
            continue;
        }

        let mut chat_timer = chat_every.map(|every| {
            let mut timer = interval(every);
            timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
            timer
        });
        let mut pending_chat: Option<(String, Instant)> = None;
        let mut chat_number = 0u32;
        let mut last_video_ts: Option<u64> = None;
        let mut seen_keyframe = false;
        let mut closed_by_peer = false;
        let mut finished = false;

        while !finished {
            tokio::select! {
                _ = sleep_until(deadline) => finished = true,
                _ = async { chat_timer.as_mut().unwrap().tick().await }, if chat_timer.is_some() => {
                    if pending_chat.take().is_some() {
                        stats.chat_lost += 1;
                    }
                    chat_number += 1;
                    let text = format!("Merge? {chat_number} de la {nick}");
                    if sink.send(Message::text(json!({ "type": "chat", "text": text }).to_string())).await.is_err() {
                        closed_by_peer = true;
                        break;
                    }
                    pending_chat = Some((text, Instant::now()));
                }
                message = incoming.next() => match message {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        closed_by_peer = true;
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                        match value["type"].as_str() {
                            Some("joined") => {
                                stats.connected = true;
                                key = value["key"].as_str().map(str::to_owned);
                            }
                            Some("chat") => {
                                if let Some((sent, at)) = &pending_chat
                                    && value["nick"] == nick
                                    && value["text"] == *sent
                                {
                                    stats.chat_rtt_ms.push(at.elapsed().as_millis() as u64);
                                    pending_chat = None;
                                }
                            }
                            // A new stream: timestamps start over, so don't count the jump as a gap.
                            Some("config") | Some("offline") => last_video_ts = None,
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Binary(frame))) => {
                        let Some(header) = FrameHeader::parse(&frame) else { continue };
                        stats.bytes += frame.len() as u64;
                        match header.kind {
                            FrameKind::Video => {
                                stats.video_frames += 1;
                                if header.keyframe {
                                    stats.keyframes += 1;
                                    if first_connection && !seen_keyframe {
                                        stats.first_keyframe_ms = Some(opened.elapsed().as_millis() as u64);
                                    }
                                    seen_keyframe = true;
                                }
                                if let Some(last) = last_video_ts
                                    && header.timestamp_us > last + GAP.as_micros() as u64
                                {
                                    stats.gaps += 1;
                                }
                                last_video_ts = Some(header.timestamp_us);
                            }
                            FrameKind::Audio => stats.audio_frames += 1,
                            FrameKind::Speech => {}
                        }
                    }
                    Some(Ok(_)) => {}
                },
            }
        }
        first_connection = false;
        if pending_chat.take().is_some() {
            stats.chat_lost += 1;
        }
        if closed_by_peer {
            stats.disconnects += 1;
            sleep_until((Instant::now() + Duration::from_secs(1)).min(deadline)).await;
        } else {
            let _ = sink.close().await;
        }
    }
    stats
}

fn median(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn summarize(all: &[Stats], duration: Duration) -> Value {
    let seconds = duration.as_secs_f64().max(0.001);
    let n = all.len() as f64;
    let video: u64 = all.iter().map(|s| s.video_frames).sum();
    let bytes: u64 = all.iter().map(|s| s.bytes).sum();
    let mut first: Vec<u64> = all.iter().filter_map(|s| s.first_keyframe_ms).collect();
    let first_max = first.iter().copied().max();
    let mut rtts: Vec<u64> = all.iter().flat_map(|s| s.chat_rtt_ms.iter().copied()).collect();
    let rtt_max = rtts.iter().copied().max();
    json!({
        "viewers": all.len(),
        "connected": all.iter().filter(|s| s.connected).count(),
        "seconds": (seconds * 10.0).round() / 10.0,
        "video_fps": ((video as f64 / n / seconds) * 10.0).round() / 10.0,
        "kbit_s": (bytes as f64 * 8.0 / n / seconds / 1000.0).round(),
        "first_keyframe_ms": { "median": median(&mut first), "max": first_max },
        "gaps": all.iter().map(|s| s.gaps).sum::<u32>(),
        "chat_rtt_ms": { "median": median(&mut rtts), "max": rtt_max },
        "chat_lost": all.iter().map(|s| s.chat_lost).sum::<u32>(),
        "disconnects": all.iter().map(|s| s.disconnects).sum::<u32>(),
        "connect_failures": all.iter().map(|s| s.connect_failures).sum::<u32>(),
    })
}

fn print_table(all: &[Stats]) {
    eprintln!(
        "{:>6} {:>7} {:>9} {:>6} {:>7} {:>10} {:>5} {:>8} {:>5}",
        "viewer", "frames", "keyframes", "audio", "MB", "1st key ms", "gaps", "chat ms", "drops"
    );
    for (i, s) in all.iter().enumerate() {
        let mut rtts = s.chat_rtt_ms.clone();
        eprintln!(
            "{:>6} {:>7} {:>9} {:>6} {:>7.1} {:>10} {:>5} {:>8} {:>5}",
            i + 1,
            s.video_frames,
            s.keyframes,
            s.audio_frames,
            s.bytes as f64 / 1e6,
            s.first_keyframe_ms.map_or("-".to_owned(), |ms| ms.to_string()),
            s.gaps,
            median(&mut rtts).map_or("-".to_owned(), |ms| ms.to_string()),
            s.disconnects + s.connect_failures,
        );
    }
}
