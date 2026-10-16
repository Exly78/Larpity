# server/

@../PROTOCOL.md

Rust 2024 with axum 0.8 and tokio. One crate, three binaries: `larpity-server`
(the default for `cargo run`), `fake-phone` and `stub-ai`. `README.md` lists the
env vars and endpoints; `deploy/README.md` covers the VPS.

## Commands (from `server/`)

- `cargo test` runs unit tests plus end-to-end tests over real WebSockets. It must pass before you push.
- `cargo clippy --all-targets -- -D warnings` must be clean.
- `cargo fmt` (CI runs `cargo fmt --check`). A hook already formats each `.rs` file Claude edits.
- `cargo run` starts the server on :8080. Use the `run-stack` skill for the whole loop.

## Where things are

- `src/protocol.rs`: every wire type (serde enums tagged by `type`, `snake_case`), the frame header, `ErrorCode`.
- `src/hub.rs`: all shared state behind one mutex: the phone and AI slots, stream config and cached frames, chat history, nicknames, ranking, speech, picked questions.
- `src/ws.rs`: the endpoints. Each connection has a reader loop plus a writer task fed by an mpsc channel, which also sends the pings.
- `src/chat.rs`, `src/ai.rs`: input checks for chat and for what the AI host sends.
- `src/recording.rs`, `src/annexb.rs`: saving streams; splitting `.h264` files for `fake-phone`.
- `tests/*.rs`: one file per area; shared helpers in `tests/common/mod.rs`.

## Rules

- Never hold the hub lock across an `.await` or while doing I/O: lock, update,
  broadcast, unlock. `join_viewer` takes its snapshot and subscribes under one
  lock so nothing is missed or sent twice; keep it that way.
- Never block on a slow client. Messages to one connection go through its mpsc
  sender with `try_send`; messages to all viewers go through the hub's broadcast.
- New message or field: add it to `protocol.rs`, document it in `PROTOCOL.md`,
  and add an integration test that checks the exact JSON.
- New error: an `ErrorCode` variant with its code and message, plus a row in
  `PROTOCOL.md`'s error table.
- New setting: a `Config` field with a default; if people should be able to set
  it, a `LARPITY_*` variable in `Config::from_env` and a row in `README.md`.
- Test behaviour end to end in `tests/`: `start(Config { .. })` a real server,
  then use the `common` helpers (`connect`, `send_json`, `expect_type`,
  `recv_binary`, `assert_quiet`, `live_stream`).
- Make timers short through `Config` (e.g. `chat_interval: Duration::ZERO`)
  rather than sleeping. Wait for a message instead of sleeping for one.
- A test that runs a binary uses `env!("CARGO_BIN_EXE_<name>")` and kills it on drop (see `Running`).
- Tests must not need ffmpeg or the network. Binary fixtures go in
  `tests/fixtures/`, the only `.h264` files git tracks.
- Keep the crate small: say why in the PR when adding a dependency.

## Gotchas

- `test-clip.h264` comes from `scripts/make-test-clip.sh` (needs ffmpeg) and is gitignored.
- Recording is off unless `LARPITY_RECORD_DIR` is set, and it uses about 0.8 GB per hour.
- On a real phone, test through the HTTPS deploy; plain `http://` on a LAN IP won't open the camera.
