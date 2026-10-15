//! The Larpity server: relays the phone's stream to viewers, runs the chat and
//! talks to the AI host. See PROTOCOL.md.

pub mod ai;
pub mod chat;
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
