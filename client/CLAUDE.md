# client/: the viewer page

@../PROTOCOL.md

Students open this on their own phones. Owners: katzow and PantedJeans71. Issues #21-#27.

**Tooling isn't chosen yet.** #21 decides it ("plain HTML and TypeScript is
enough"). Until then don't add a framework, bundler or `package.json` on your own.
When #21 lands, replace this paragraph with the build and check commands.

## Rules

- The server serves this folder as static files at `/` (`LARPITY_CLIENT_DIR`). Whatever
  the tooling, the result must be plain files the server can serve; no dev server in production.
- Connect on the same origin and never hardcode a host:
  ```ts
  new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws/viewer`)
  ```
- Video: configure `VideoDecoder` from the `config` message with only `codec` and
  `optimizeForLatency: true` (Annex B, so no `description`). Create
  `EncodedVideoChunk`s as `"key"` when the keyframe flag is set, otherwise
  `"delta"`. On `config` or `offline`, reset the decoder and drop frames until the next keyframe.
- Audio: `pcm16` is 16 kHz mono 16-bit little-endian. Turn it into `AudioBuffer`s and
  schedule them back to back. iPhone only starts an `AudioContext` from a user tap,
  so show a "tap to listen" button.
- Chat: show only the `chat` messages the server sends back, never the user's own
  message locally. Show errors by mapping `error.code` to Romanian text (the
  server's `message` is English, for developers).
- The "being answered" banner comes from `question_picked`.
- Reconnect when the socket closes, backing off 1 s, 2 s, 4 s, then every 10 s.
  The server replays config, frames, chat and questions on connect, so start from a clean state.
- Lay it out for a phone held upright first (about 360 px wide). Test on Android
  Chrome and iPhone Safari before calling a UI change done.
- All UI text is Romanian; keep the strings in one place so a teammate can proofread them.

## Trying it

Run the server, `fake-phone` and `stub-ai` with the `run-stack` skill, then open
<http://localhost:8080/>. A phone needs HTTPS, so use the deployed test server.
