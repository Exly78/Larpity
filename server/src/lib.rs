//! The Larpity server: relays the phone's stream to viewers. See PROTOCOL.md.

pub mod config;
pub mod hub;
pub mod protocol;
pub mod recording;
mod ws;

pub use config::Config;

/// The whole app, ready for `axum::serve`.
pub fn app(config: Config) -> axum::Router {
    ws::router(config)
}
