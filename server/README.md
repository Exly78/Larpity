# server

The Rust server. It relays the phone's stream to every viewer, runs the chat
room, and talks to the AI host. The message format is in
[`../PROTOCOL.md`](../PROTOCOL.md).

## Run it

```sh
cd server
cargo run --release
```

Then open <http://localhost:8080/> for the viewer page (`client/`) and
<http://localhost:8080/phone/> for the phone page (`firmware/`). Browsers only
allow the camera and WebCodecs on `localhost` or HTTPS, so test on a real phone
through the deployed server (see [`deploy/`](deploy/)).

Settings come from environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `LARPITY_ADDR` | `0.0.0.0:8080` | address to listen on |
| `LARPITY_TOKEN` | unset | secret the phone and AI host pass as `?token=`; unset lets anyone in |
| `LARPITY_RECORD_DIR` | unset | save every stream here; unset turns recording off |
| `LARPITY_CLIENT_DIR` | `../client` | served at `/` |
| `LARPITY_FIRMWARE_DIR` | `../firmware` | served at `/phone/` |
| `RUST_LOG` | `info` | log level, e.g. `debug` |

## Recordings

With `LARPITY_RECORD_DIR` set, each stream (each `config` from the phone) is
saved as it arrives:

- `larpity-<unix time>.h264`: the video, exactly as the phone sent it
- `larpity-<unix time>.pcm`: the audio, when it is `pcm16`
- `larpity-<unix time>.json`: the stream config

Turn them into an MP4:

```sh
ffmpeg -framerate 15 -i larpity-1760520000.h264 \
       -f s16le -ar 16000 -ac 1 -i larpity-1760520000.pcm \
       -c:v copy -c:a aac larpity-1760520000.mp4
```

The raw `.h264` has no timestamps, so ffmpeg assumes a steady 15 fps. If the
phone dropped frames, sound and picture drift apart over a long recording.

## Endpoints

| Path | What |
|---|---|
| `/ws/phone?token=…` | the phone |
| `/ws/viewer` | viewers |
| `/phone/` | the phone page |
| `/` | the viewer page |
| `/health` | returns `ok` |

## Develop

```sh
cargo test                              # unit tests + end-to-end tests over real WebSockets
cargo clippy --all-targets -- -D warnings
cargo fmt
```

CI runs the same three on every pull request that touches `server/`.

## How it works

- `src/protocol.rs`: the binary frame header and the JSON messages.
- `src/hub.rs`: shared state. Which connection is the phone, the current stream
  config, and the frames since the last keyframe, which a viewer who joins late
  gets first so the picture appears at once. Video fans out to viewers over one
  broadcast channel; a viewer that falls too far behind skips to the next
  keyframe instead of being disconnected.
- `src/ws.rs`: one reader loop and one writer task per connection. The writer
  pings every 5 s; a connection that is silent for 15 s is dropped.
- `src/recording.rs`: writes the stream to disk, flushing every frame.
- `src/chat.rs`: chat rules: cleaning nicknames and messages, and numbering
  taken nicknames. The hub keeps the last 50 messages for late joiners.
