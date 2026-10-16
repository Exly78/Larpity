use std::{env, path::PathBuf, time::Duration};

#[derive(Debug, Clone)]
pub struct Config {
    /// Shared secret the phone and AI host must pass as `?token=`. `None` lets anyone in (local testing).
    pub token: Option<String>,
    /// Where to save each stream; `None` turns recording off.
    pub record_dir: Option<PathBuf>,
    /// Served at `/`.
    pub client_dir: PathBuf,
    /// Served at `/phone/`.
    pub firmware_dir: PathBuf,
    pub ping_every: Duration,
    /// A connection that sends nothing (not even a pong) for this long is dropped.
    pub idle_timeout: Duration,
    /// The shortest gap allowed between two chat messages from one viewer.
    pub chat_interval: Duration,
    /// Button presses closer together than this count as one, so a double tap doesn't skip a question.
    pub button_debounce: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: None,
            record_dir: None,
            client_dir: PathBuf::from("../client"),
            firmware_dir: PathBuf::from("../firmware"),
            ping_every: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(15),
            chat_interval: Duration::from_secs(1),
            button_debounce: Duration::from_secs(1),
        }
    }
}

impl Config {
    /// Reads `LARPITY_*` environment variables, falling back to the defaults.
    pub fn from_env() -> Self {
        let mut config = Self {
            token: env::var("LARPITY_TOKEN").ok().filter(|t| !t.is_empty()),
            record_dir: env::var_os("LARPITY_RECORD_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
            ..Self::default()
        };
        if let Ok(dir) = env::var("LARPITY_CLIENT_DIR") {
            config.client_dir = dir.into();
        }
        if let Ok(dir) = env::var("LARPITY_FIRMWARE_DIR") {
            config.firmware_dir = dir.into();
        }
        config
    }

    pub fn authorized(&self, given: Option<&str>) -> bool {
        match (&self.token, given) {
            (None, _) => true,
            (Some(expected), Some(given)) => constant_time_eq(expected.as_bytes(), given.as_bytes()),
            (Some(_), None) => false,
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_check() {
        let open = Config::default();
        assert!(open.authorized(None));
        assert!(open.authorized(Some("anything")));

        let locked = Config {
            token: Some("secret".into()),
            ..Config::default()
        };
        assert!(locked.authorized(Some("secret")));
        assert!(!locked.authorized(Some("secreT")));
        assert!(!locked.authorized(Some("secret2")));
        assert!(!locked.authorized(None));
    }
}
