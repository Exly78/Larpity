//! Pretends to be the phone: streams an H.264 clip to the server on a loop, with
//! a 440 Hz tone as the audio, so the viewer page can be built and tested
//! without a real phone. Press Enter to press the "next question" button.
//!
//!     cargo run --bin fake-phone -- --clip test-clip.h264 [--url ws://127.0.0.1:8080/ws/phone] [--token SECRET]
//!
//! Make a clip with scripts/make-test-clip.sh, or turn a phone recording into one:
//!
//!     ffmpeg -i clip.mp4 -an -c:v copy -bsf:v h264_mp4toannexb -f h264 clip.h264
//!
//! It reconnects when the connection drops, and stops for good when a real
//! phone takes over (close code 4000).

use std::{env, f32::consts::TAU, path::PathBuf, process::exit, time::Duration};

use futures_util::{SinkExt, StreamExt};
use larpity_server::{
    annexb::{self, AccessUnit},
    protocol::{FrameHeader, FrameKind, HEADER_LEN, encode_frame},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const USAGE: &str = "usage: fake-phone --clip FILE.h264 [options]

  --clip FILE   H.264 Annex B clip to stream (see scripts/make-test-clip.sh)
  --url URL     the server's phone endpoint (default ws://127.0.0.1:8080/ws/phone)
  --token T     the server's LARPITY_TOKEN (default: the LARPITY_TOKEN variable)
  --fps N       frames per second (default 15)
  --no-audio    don't send the test tone
  --once        play the clip once and stop instead of looping

Press Enter to send a button press.";

const SAMPLE_RATE: u32 = 16_000;
const AUDIO_FRAME: Duration = Duration::from_millis(20);
const SAMPLES_PER_FRAME: u32 = SAMPLE_RATE / 50;

struct Args {
    url: String,
    clip: PathBuf,
    fps: f64,
    audio: bool,
    once: bool,
}

enum End {
    /// The connection dropped; try again.
    Closed,
    /// A real phone took over.
    Replaced,
    /// `--once` and the clip is done.
    Finished,
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let data = std::fs::read(&args.clip).unwrap_or_else(|err| {
        eprintln!("can't read {}: {err}", args.clip.display());
        exit(1)
    });
    // Start at the first keyframe so viewers can decode from the first frame sent.
    let units: Vec<AccessUnit> = annexb::access_units(&data)
        .into_iter()
        .skip_while(|unit| !unit.keyframe)
        .collect();
    if units.is_empty() {
        eprintln!("{} has no keyframe: is it an H.264 Annex B file?", args.clip.display());
        exit(1)
    }
    let info = annexb::stream_info(&data).unwrap_or_else(|| {
        eprintln!("couldn't read the clip's SPS; assuming 1280x720 avc1.42E01F");
        annexb::StreamInfo {
            codec: "avc1.42E01F".into(),
            width: 1280,
            height: 720,
        }
    });
    let keyframes = units.iter().filter(|unit| unit.keyframe).count();
    eprintln!(
        "{}: {} frames ({:.1} s at {} fps), {keyframes} keyframes, {} {}x{}",
        args.clip.display(),
        units.len(),
        units.len() as f64 / args.fps,
        args.fps,
        info.codec,
        info.width,
        info.height,
    );
    let config = json!({
        "type": "config",
        "video": { "codec": info.codec, "width": info.width, "height": info.height, "framerate": args.fps },
        "audio": if args.audio { json!({ "codec": "pcm16", "sample_rate": SAMPLE_RATE, "channels": 1 }) } else { Value::Null },
    });

    let mut presses = button_presses();
    let mut clock = Clock::default();
    loop {
        match stream(&args, &units, &config, &mut presses, &mut clock).await {
            Ok(End::Finished) => return,
            Ok(End::Replaced) => {
                eprintln!("another phone took over; stopping");
                return;
            }
            Ok(End::Closed) => eprintln!("the server closed the connection"),
            Err(err) => eprintln!("connection failed: {err}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        eprintln!("reconnecting to {}", args.url);
    }
}

fn parse_args() -> Args {
    let mut url = "ws://127.0.0.1:8080/ws/phone".to_owned();
    let mut token = env::var("LARPITY_TOKEN").ok().filter(|t| !t.is_empty());
    let mut clip = None;
    let mut fps = 15.0;
    let mut audio = true;
    let mut once = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{arg} needs a value\n\n{USAGE}");
                exit(2)
            })
        };
        match arg.as_str() {
            "--url" => url = value(),
            "--token" => token = Some(value()),
            "--clip" => clip = Some(PathBuf::from(value())),
            "--fps" => {
                fps = value()
                    .parse()
                    .ok()
                    .filter(|f: &f64| *f > 0.0 && *f <= 120.0)
                    .unwrap_or_else(|| {
                        eprintln!("--fps needs a number between 0 and 120");
                        exit(2)
                    })
            }
            "--no-audio" => audio = false,
            "--once" => once = true,
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
    let Some(clip) = clip else {
        eprintln!("--clip is required\n\n{USAGE}");
        exit(2)
    };
    if let Some(token) = token {
        url = format!("{url}{}token={token}", if url.contains('?') { '&' } else { '?' });
    }
    Args {
        url,
        clip,
        fps,
        audio,
        once,
    }
}

/// One message per line typed on stdin. Nothing arrives when stdin isn't a terminal.
fn button_presses() -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(_)) = lines.next_line().await {
            if tx.send(()).await.is_err() {
                break;
            }
        }
    });
    rx
}

/// Timestamps keep counting up across reconnects, like a real phone's clock.
#[derive(Default)]
struct Clock {
    video_frames: u64,
    audio_frames: u64,
}

impl Clock {
    fn next_video_us(&mut self, fps: f64) -> u64 {
        let ts = (self.video_frames as f64 * 1_000_000.0 / fps) as u64;
        self.video_frames += 1;
        ts
    }

    /// The next 20 ms of a 440 Hz tone as 16-bit little-endian PCM, and its timestamp.
    fn next_audio(&mut self) -> (u64, Vec<u8>) {
        let first = self.audio_frames * u64::from(SAMPLES_PER_FRAME);
        self.audio_frames += 1;
        let pcm = (0..u64::from(SAMPLES_PER_FRAME))
            .flat_map(|i| {
                let t = (first + i) as f32 / SAMPLE_RATE as f32;
                let sample = (t * 440.0 * TAU).sin() * 0.2 * f32::from(i16::MAX);
                (sample as i16).to_le_bytes()
            })
            .collect();
        (first * 1_000_000 / u64::from(SAMPLE_RATE), pcm)
    }
}

async fn stream(
    args: &Args,
    units: &[AccessUnit<'_>],
    config: &Value,
    presses: &mut mpsc::Receiver<()>,
    clock: &mut Clock,
) -> Result<End, tokio_tungstenite::tungstenite::Error> {
    let (ws, _) = connect_async(&args.url).await?;
    eprintln!("connected; streaming (press Enter for the button)");
    let (mut sink, mut incoming) = ws.split();
    sink.send(Message::text(config.to_string())).await?;

    let mut video_timer = interval(Duration::from_secs_f64(1.0 / args.fps));
    video_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut audio_timer = interval(AUDIO_FRAME);
    audio_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut next = 0;
    let mut stdin_open = true;

    loop {
        tokio::select! {
            _ = video_timer.tick() => {
                if next == units.len() {
                    if args.once {
                        sink.close().await?;
                        return Ok(End::Finished);
                    }
                    next = 0;
                }
                let unit = &units[next];
                next += 1;
                let frame = encode_frame(FrameKind::Video, unit.keyframe, clock.next_video_us(args.fps), unit.data);
                sink.send(Message::binary(frame.to_vec())).await?;
            }
            _ = audio_timer.tick(), if args.audio => {
                let (ts, pcm) = clock.next_audio();
                sink.send(Message::binary(encode_frame(FrameKind::Audio, false, ts, &pcm).to_vec())).await?;
            }
            press = presses.recv(), if stdin_open => match press {
                Some(()) => {
                    sink.send(Message::text(json!({ "type": "button" }).to_string())).await?;
                    eprintln!("button pressed");
                }
                None => stdin_open = false,
            },
            message = incoming.next() => match message {
                None => return Ok(End::Closed),
                Some(Err(err)) => return Err(err),
                Some(Ok(Message::Close(Some(frame)))) if u16::from(frame.code) == 4000 => return Ok(End::Replaced),
                Some(Ok(Message::Close(_))) => return Ok(End::Closed),
                Some(Ok(Message::Text(text))) => show(&text),
                Some(Ok(Message::Binary(frame))) => {
                    if FrameHeader::parse(&frame).is_some_and(|h| h.kind == FrameKind::Speech) {
                        eprintln!("  speech: {} bytes of audio", frame.len() - HEADER_LEN);
                    }
                }
                Some(Ok(_)) => {}
            },
        }
    }
}

fn show(text: &str) {
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        eprintln!("server: {text}");
        return;
    };
    match message["type"].as_str() {
        Some("question_picked") => eprintln!(
            "question picked: {} (speech {})",
            message["text"],
            if message["audio"] == true {
                "follows"
            } else {
                "not ready"
            }
        ),
        Some("error") => eprintln!("server error {}: {}", message["code"], message["message"]),
        _ => eprintln!("server: {text}"),
    }
}
