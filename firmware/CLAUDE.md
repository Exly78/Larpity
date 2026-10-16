# firmware/: the phone page

@../PROTOCOL.md

Runs in the browser on the phone the professional wears. Owners: Ski and Exly.
Issues #13-#20 (device) and #6, #9 (audio).

**Tooling isn't chosen yet.** Until #13 decides it, don't add a framework, bundler
or `package.json` on your own. When it lands, put the build and check commands here.

## Rules

- The server serves this folder at `/phone/` (`LARPITY_FIRMWARE_DIR`). The page is
  opened as `https://<host>/phone/?token=<token>`: read the token from
  `location.search` and add it to `/ws/phone?token=`. Never hardcode or commit a token.
- Build the WebSocket URL from `location` (same origin, `wss` on HTTPS).
- Send `config` first, then frames. The first frame and every 30th are keyframes:
  `encoder.encode(frame, { keyFrame: n % 30 === 0 })`. Configure the encoder with
  `avc: { format: "annexb" }` and `latencyMode: "realtime"`. The header timestamp is
  `VideoFrame.timestamp` (µs).
- Don't build a backlog on a slow network. If `ws.bufferedAmount` keeps growing,
  skip frames until the next keyframe rather than sending late video.
- Close code 4000 means another phone took over: stop, say so on screen, and don't
  reconnect. Any other close: reconnect with backoff, send `config` again, and start
  with a keyframe.
- Speech comes as `question_picked` followed by a kind-3 frame. Play it with
  `AudioContext.decodeAudioData`. When `audio` is `false`, show the text instead.
- Keep the screen awake with the Screen Wake Lock API, and get it back on
  `visibilitychange` (#19).
- iPhone needs a user tap before the camera, mic or audio playback start, so begin with a big "Start" button.
- The person wearing the phone is working: big, high-contrast controls, little
  text, and Romanian UI.
- Test on Android Chrome and iPhone Safari. If you can't, say so in the PR and list
  the steps for a person to run.

## Trying it

Run the stack with the `run-stack` skill and open
<http://localhost:8080/phone/?token=dev> on a laptop with a webcam. A real phone
needs HTTPS, so use the deployed test server.
