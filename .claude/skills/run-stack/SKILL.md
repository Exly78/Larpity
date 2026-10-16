---
name: run-stack
description: Run the whole Larpity stack locally (server, fake phone streaming a test clip, stub AI host) to check a change end to end. Use to confirm streaming, chat, the question ranking or the button really works, or when asked to run, start or demo the app.
---

# Run the stack locally

Everything runs from `server/`. The token `dev` is only for local runs.

## Start it

```sh
cd server
[ -f test-clip.h264 ] || scripts/make-test-clip.sh      # once; needs ffmpeg
cargo build --release
LARPITY_TOKEN=dev target/release/larpity-server &        # http://localhost:8080
target/release/stub-ai --token dev &                     # ranks chat questions, beeps as speech
target/release/fake-phone --clip test-clip.h264 --token dev   # Enter = button press
```

- When the shell can't take keyboard input (Claude's tool, scripts), press the
  button by piping a newline in after a delay:
  `(sleep 5; echo; sleep 3) | target/release/fake-phone --clip test-clip.h264 --token dev`
- If port 8080 is taken, set `LARPITY_ADDR=127.0.0.1:8081` and pass
  `--url ws://127.0.0.1:8081/ws/...` to both tools.
- To check recording, add `LARPITY_RECORD_DIR=/tmp/larpity-rec` to the server.
- To stream another clip, convert it first:
  `ffmpeg -i in.mp4 -an -c:v copy -bsf:v h264_mp4toannexb -f h264 clip.h264`.

## Look at it

- Viewer page: <http://localhost:8080/>. Phone page: <http://localhost:8080/phone/?token=dev>.
- Without a browser, use a quick viewer in Node 22 (it has a built-in `WebSocket`):

```js
// node viewer.mjs  (save it outside the repo)
const ws = new WebSocket("ws://127.0.0.1:8080/ws/viewer");
ws.binaryType = "arraybuffer";
let video = 0, keys = 0, audio = 0;
ws.onopen = () => {
  ws.send(JSON.stringify({ type: "join", nick: "Test" }));
  setTimeout(() => ws.send(JSON.stringify({ type: "chat", text: "Ce faci azi?" })), 1000);
};
ws.onmessage = (e) => {
  if (typeof e.data === "string") return console.log(e.data.slice(0, 160));
  const v = new DataView(e.data);
  if (v.getUint8(0) === 1) { video++; if (v.getUint8(1) & 1) keys++; } else audio++;
};
setTimeout(() => { console.log({ video, keys, audio }); process.exit(0); }, 6000);
```

## What working looks like

- The viewer gets `config`, then about 15 video frames a second with a keyframe
  every 2 s, and about 50 audio frames a second.
- A chat message with a `?` comes back as a `questions` list within a moment.
- Enter in `fake-phone` prints `question picked: ... (speech follows)` and the
  size of the speech, and viewers get `question_picked`.

## Clean up

Stop everything you started before finishing, and don't leave a server running:

```sh
pkill -x 'larpity-server|stub-ai|fake-phone'
```

Match process names with `-x`. `pkill -f` matches the whole command line, so it
also kills the shell running it.

This can't show how things behave on a real phone (camera, iPhone audio, heat,
mobile data). That needs HTTPS and the deployed server (`server/deploy/README.md`).
