//! Saves the phone's stream to disk as it arrives, and keeps a log of the chat
//! and the questions.
//!
//! For each stream there is `larpity-<unix time>.json` (the config), `.h264` (the
//! Annex B video, playable as is) and, when the audio is `pcm16`, `.pcm` (raw
//! 16-bit samples). Every write is flushed, so a crash loses nothing already received.
//!
//! Next to them, `chat-<unix time>.jsonl` and `questions-<unix time>.jsonl` get
//! one JSON line per chat message and per question event for the whole server
//! run, so a session can be looked at afterwards (what was asked, in what
//! order, what got answered when).

use std::{
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    sync::mpsc,
};
use tracing::warn;

use crate::protocol::StreamConfig;

/// Which log a line goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFile {
    /// Every `chat` message, exactly as viewers got it.
    Chat,
    /// Every `questions` list sent to viewers and every `question_picked`, with a `ts`.
    Questions,
}

/// Appends lines to the two `.jsonl` files from its own task, so the hub never
/// does file I/O under its lock. A line is dropped (with a warning) if the
/// queue is full or the file can't be written; the stream is never held up.
#[derive(Clone)]
pub struct EventLog {
    tx: mpsc::Sender<(LogFile, String)>,
}

impl EventLog {
    /// Starts the writer. The files are created on the first line, named by `dir` and the current time.
    pub fn spawn(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(write_log_lines(dir, rx));
        Self { tx }
    }

    pub fn log(&self, file: LogFile, line: String) {
        if self.tx.try_send((file, line)).is_err() {
            warn!(?file, "event log is backed up; dropping a line");
        }
    }
}

async fn write_log_lines(dir: PathBuf, mut rx: mpsc::Receiver<(LogFile, String)>) {
    let stamp = unix_secs();
    // `None` until the first line; `Some(Err)` once writing failed, so it is not retried.
    let mut files: [Option<io::Result<File>>; 2] = [None, None];
    while let Some((which, line)) = rx.recv().await {
        let slot = &mut files[which as usize];
        if slot.is_none() {
            let name = match which {
                LogFile::Chat => format!("chat-{stamp}.jsonl"),
                LogFile::Questions => format!("questions-{stamp}.jsonl"),
            };
            let path = dir.join(name);
            let opened = async {
                fs::create_dir_all(&dir).await?;
                OpenOptions::new().append(true).create(true).open(&path).await
            }
            .await;
            if let Err(err) = &opened {
                warn!(path = %path.display(), %err, "can't write the event log");
            }
            *slot = Some(opened);
        }
        let Some(Ok(file)) = slot else { continue };
        let result = async {
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await?;
            file.flush().await
        }
        .await;
        if let Err(err) = result {
            warn!(?which, %err, "event log failed; stopping it");
            *slot = Some(Err(err));
        }
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Recorder {
    base: PathBuf,
    video: File,
    audio: Option<File>,
}

impl Recorder {
    pub async fn start(dir: &Path, config: &StreamConfig) -> io::Result<Self> {
        fs::create_dir_all(dir).await?;
        let stamp = unix_secs();
        // Two streams can start in the same second (a reconnect); number them apart.
        let (base, video) = {
            let mut attempt = 1;
            loop {
                let name = match attempt {
                    1 => format!("larpity-{stamp}"),
                    n => format!("larpity-{stamp}-{n}"),
                };
                let base = dir.join(name);
                match create_new(&base.with_extension("h264")).await {
                    Ok(file) => break (base, file),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
                    Err(err) => return Err(err),
                }
            }
        };
        let audio = match &config.audio {
            Some(audio) if audio.codec == "pcm16" => Some(create_new(&base.with_extension("pcm")).await?),
            _ => None,
        };
        let json = serde_json::to_vec_pretty(config).expect("config always serializes");
        fs::write(base.with_extension("json"), json).await?;
        Ok(Self { base, video, audio })
    }

    /// Where the files go, without the extension.
    pub fn base(&self) -> &Path {
        &self.base
    }

    pub async fn video(&mut self, payload: &[u8]) -> io::Result<()> {
        self.video.write_all(payload).await?;
        self.video.flush().await
    }

    pub async fn audio(&mut self, payload: &[u8]) -> io::Result<()> {
        match &mut self.audio {
            Some(file) => {
                file.write_all(payload).await?;
                file.flush().await
            }
            None => Ok(()),
        }
    }
}

async fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path).await
}
