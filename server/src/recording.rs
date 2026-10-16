//! Saves the phone's stream to disk as it arrives.
//!
//! For each stream there is `larpity-<unix time>.json` (the config), `.h264` (the
//! Annex B video, playable as is) and, when the audio is `pcm16`, `.pcm` (raw
//! 16-bit samples). Every write is flushed, so a crash loses nothing already received.

use std::{
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

use crate::protocol::StreamConfig;

pub struct Recorder {
    base: PathBuf,
    video: File,
    audio: Option<File>,
}

impl Recorder {
    pub async fn start(dir: &Path, config: &StreamConfig) -> io::Result<Self> {
        fs::create_dir_all(dir).await?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
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
