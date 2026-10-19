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
through the deployed server (see [`deploy/`](deploy/README.md)).

## Test without a phone

`fake-phone` streams an H.264 clip on a loop, exactly as the phone would, with
a 440 Hz tone as the audio. Press Enter in its terminal to press the button.

```sh
scripts/make-test-clip.sh                        # once; needs ffmpeg
cargo run --release &                            # the server
cargo run --release --bin fake-phone -- --clip test-clip.h264
```

To stream Ski's sample clip instead, turn it into a raw H.264 file first:

```sh
ffmpeg -i sample.mp4 -an -c:v copy -bsf:v h264_mp4toannexb -f h264 sample.h264
```

A recording from the server (`larpity-<time>.h264`) can be streamed as it is.

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

And for the whole server run, one JSON line per event:

- `chat-<unix time>.jsonl`: every `chat` message, exactly as viewers got it
- `questions-<unix time>.jsonl`: every `questions` list sent to viewers and
  every `question_picked`, each with a `ts` (Unix ms) in front

So after a session you can see what was asked, in what order, and what got
answered when. Chat happens while the phone is offline too, so these files
are per server run, not per stream.

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
| `/ws/ai?token=…` | the AI host |
| `/phone/` | the phone page |
| `/` | the viewer page |
| `/health` | returns `ok` |
| `/status` | is it working? JSON, see below |

The two pages are served with `Cache-Control: no-cache`, so browsers check for
a new version on every load (one small request per file, thanks to the ETag)
and a deploy reaches every viewer at once. `CLAUDE.md`, `README.md`,
`package.json`, `tsconfig.json`, `src/` and dotfiles in those folders are not
served.

## Status

`/status` answers "is it working?" from any phone, without the server logs:

```json
{ "uptime_s": 1234, "phone": true,
  "stream": { "video": { "codec": "avc1.42E01F", "width": 1280, "height": 720, "framerate": 15 },
              "audio": { "codec": "pcm16", "sample_rate": 16000, "channels": 1 } },
  "recording": "/var/lib/larpity/recordings/larpity-1760520000",
  "viewers": 12, "ai": true, "questions": 4, "picked": 2, "chat_messages": 37 }
```

`stream` and `recording` are `null` while no phone is streaming. `questions` is
the queue the button picks from; `picked` and `chat_messages` count since the
server started. The page needs no token: it holds nothing secret.

## Test without the AI host

`stub-ai` is a stand-in AI host: every chat message with a `?` becomes a
question, repeats are merged with a count, and each question gets a beep as
its speech.

```sh
cargo run --bin stub-ai -- --url ws://localhost:8080/ws/ai --token "$LARPITY_TOKEN"
```

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
- `src/ai.rs`: checks rankings and speech from the AI host. The hub keeps
  the latest ranking and a speech clip per question, and forwards every chat
  message to the AI host.
- `src/annexb.rs`: splits a `.h264` file into frames and reads its size and
  codec string, for `fake-phone`.
- `src/chat.rs`: chat rules: cleaning nicknames and messages, and numbering
  taken nicknames. The hub keeps the last 50 messages for late joiners.
