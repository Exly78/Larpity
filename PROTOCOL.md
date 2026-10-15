# Larpity message format

Everything goes through the server over WebSockets. There are three kinds of
connection:

| Endpoint | Who connects | How many | Auth |
|---|---|---|---|
| `/ws/phone?token=<token>` | the phone page in `firmware/` | one at a time | token |
| `/ws/viewer` | the viewer page in `client/` | any number | none |
| `/ws/ai?token=<token>` | the AI host | one at a time | token |

A second phone (or AI host) that connects takes over from the first one, which
the server then disconnects. This is how a phone reconnects after switching
between Wi-Fi and mobile data: it just connects again.

Every connection carries two kinds of WebSocket message:

- **Text messages** are JSON objects with a `"type"` field. Unknown fields are
  ignored, so new fields can be added later without breaking anyone.
- **Binary messages** are media frames: video, audio, or speech.

The server pings every connection every 5 seconds (browsers answer
automatically) and drops a connection it hasn't heard anything from for 15
seconds.

## Binary frames

Every binary message starts with a 10-byte header, then the payload:

| Offset | Size | Field | Meaning |
|---|---|---|---|
| 0 | 1 | `kind` | `1` video, `2` audio, `3` speech |
| 1 | 1 | `flags` | bit 0 set = keyframe (video only); other bits are 0 |
| 2 | 8 | `timestamp` | microseconds, unsigned, big-endian |
| 10 | rest | payload | see below |

```js
// Building a frame in the browser
function frame(kind, keyframe, timestampUs, payload /* Uint8Array */) {
  const out = new Uint8Array(10 + payload.byteLength);
  const view = new DataView(out.buffer);
  view.setUint8(0, kind);
  view.setUint8(1, keyframe ? 1 : 0);
  view.setBigUint64(2, BigInt(Math.round(timestampUs)));
  out.set(payload, 10);
  return out;
}

// Reading one
const view = new DataView(data); // data: ArrayBuffer from the socket
const kind = view.getUint8(0);
const keyframe = (view.getUint8(1) & 1) === 1;
const timestampUs = Number(view.getBigUint64(2));
const payload = new Uint8Array(data, 10);
```

### Video (`kind` 1)

- H.264 in **Annex B** format (start codes, with SPS and PPS inside every
  keyframe). Annex B means a viewer can configure its decoder from the `config`
  message alone, with no separate `description`, and a recording is just the
  payloads written one after another into a `.h264` file.
- 1280×720 at 15 fps. Codec string `avc1.42E01F` (Constrained Baseline, level
  3.1) unless testing shows a phone needs something else.
- One payload is one `EncodedVideoChunk`, i.e. one whole frame.
- **A keyframe every 2 seconds** (every 30 frames), and the very first frame
  must be a keyframe. Viewers that join late start from the most recent
  keyframe, so this sets how long they wait.

```js
// Phone side
encoder.configure({
  codec: "avc1.42E01F",
  width: 1280, height: 720, framerate: 15,
  bitrate: 1_500_000,            // pick the real value in issue #14
  latencyMode: "realtime",
  avc: { format: "annexb" },
});
encoder.encode(videoFrame, { keyFrame: frameCount % 30 === 0 });

// Viewer side: no description needed for Annex B
decoder.configure({ codec: config.video.codec, optimizeForLatency: true });
```

### Audio (`kind` 2)

- `pcm16`: signed 16-bit little-endian samples, mono, 16 000 Hz.
- 20 ms per frame (320 samples, 640 bytes) is recommended; any length works.
- The timestamp is on the same clock as the video, so viewers can line them up.

Raw PCM is about 256 kbit/s, small next to the video, and every browser can
play it with Web Audio without needing an audio decoder. The `config` message
names the codec, so we can switch to Opus later without changing anything else.

### Speech (`kind` 3)

Server → phone only. The payload is a whole audio file (MP3, WAV, or whatever
the TTS provider returns; the format is named in the `question_picked` message
just before it). Play it with `AudioContext.decodeAudioData`. The timestamp is
the server's clock and can be ignored.

## Phone ↔ server (`/ws/phone`)

Right after connecting, the phone sends `config`. The server ignores media
frames until it has a `config`. Send `config` again if the video settings
change; viewers reset their decoders when they get a new one.

**Phone → server**

```jsonc
{ "type": "config",
  "video": { "codec": "avc1.42E01F", "width": 1280, "height": 720, "framerate": 15 },
  "audio": { "codec": "pcm16", "sample_rate": 16000, "channels": 1 } }   // "audio" may be null
{ "type": "button" }   // the "next question" button
```

Plus binary video (`kind` 1) and audio (`kind` 2) frames.

**Server → phone**

```jsonc
{ "type": "question_picked", "id": "q7", "text": "Cât durează o operație?", "audio": true, "format": "mp3" }
{ "type": "error", "code": "no_question", "message": "No questions in the queue yet" }
```

When `audio` is `true`, a speech frame (`kind` 3) follows right after. When it
is `false` the AI hasn't finished the speech yet; the phone can show the text on
screen instead.

Button presses less than a second apart are treated as one press, so a double
tap doesn't skip a question.

## Viewer ↔ server (`/ws/viewer`)

On connect the server sends, in order:

1. `config` if the stream is live, or `offline` if it isn't.
2. If live: video frames starting from the latest keyframe, so the picture
   appears immediately.
3. The last 50 chat messages, as `chat` messages.
4. The current `questions` list, and the current `question_picked` if one is
   being answered.

After that it streams everything live.

**Viewer → server**

```jsonc
{ "type": "join", "nick": "Ana" }      // required before chatting; send again to change nickname
{ "type": "chat", "text": "Ce ai studiat?" }
```

**Server → viewer**

```jsonc
{ "type": "config", "video": { ... }, "audio": { ... } }   // same as the phone's; (re)configure the decoder
{ "type": "offline" }                                       // the phone disconnected; reset and wait for config
{ "type": "joined", "nick": "Ana2" }                        // your nickname, possibly with a number added if taken
{ "type": "chat", "id": 12, "nick": "Ana", "text": "Ce ai studiat?", "ts": 1760520000000 }  // ts = Unix ms
{ "type": "questions", "questions": [ { "id": "q7", "text": "Cât durează o operație?", "count": 4 } ] }  // best first
{ "type": "question_picked", "id": "q7", "text": "Cât durează o operație?" }   // show the "being answered" banner
{ "type": "error", "code": "rate_limited", "message": "..." }
```

Plus binary video (`kind` 1) and audio (`kind` 2) frames, exactly as the phone
sent them.

Chat rules: nicknames are 1–24 characters and unique among connected viewers
(the server adds a number if one is taken). Messages are 1–200 characters, at
most one per second per viewer. Every viewer, including the sender, gets each
`chat` message back from the server, so only render what comes from the server.

If a viewer falls behind (slow network), the server skips video until the next
keyframe instead of disconnecting it.

## AI host ↔ server (`/ws/ai`)

The AI host is a separate program in any language. It connects to the server,
reads the chat, and sends back rankings and speech. On connect the server sends
the last 50 chat messages so a restarted AI host catches up.

**Server → AI**

```jsonc
{ "type": "chat", "id": 12, "nick": "Ana", "text": "Ce ai studiat?", "ts": 1760520000000 }
{ "type": "question_picked", "id": "q7" }   // the professional is answering this one; drop it from the ranking
```

**AI → server**

```jsonc
// The full ranked list, best first, replacing the previous one. Up to 20 questions.
// id: your own stable id for the question, so speech can refer to it.
// text: one short spoken sentence. count: how many chat messages asked it.
{ "type": "ranking", "questions": [
    { "id": "q7", "text": "Cât durează o operație?", "count": 4, "chat_ids": [12, 15, 19, 20] } ] }

// Speech for one question, generated as soon as it is ranked (issue #31), so the
// button press can play it at once. audio is base64; at most 2 MB once decoded.
{ "type": "speech", "question_id": "q7", "format": "mp3", "audio": "SUQzBAAAAAAA..." }
```

`chat_ids` is optional. The server forwards `ranking` to viewers as
`questions` (without `chat_ids`), never re-lists a question that has already
been picked, and keeps the last ranking and speech if the AI host disconnects.

## Error codes

| Code | Sent to | Meaning |
|---|---|---|
| `bad_message` | anyone | not valid JSON, or an unknown `type` |
| `bad_frame` | phone | binary message shorter than 10 bytes or an unknown `kind` |
| `config_required` | phone | media sent before `config` |
| `no_question` | phone | button pressed with nothing in the queue |
| `join_required` | viewer | `chat` sent before `join` |
| `bad_nick` | viewer | nickname empty or too long |
| `bad_text` | viewer | message empty or too long |
| `rate_limited` | viewer | more than one message per second |
| `bad_speech` | AI | `audio` isn't valid base64 or is too big |
