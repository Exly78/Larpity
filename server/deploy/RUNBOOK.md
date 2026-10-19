# Running a session

The checklist for the rehearsal and the pilot. `README.md` next to this file
covers setting the VPS up; this is the day itself. Everything here is run as
root on the VPS unless it says otherwise.

## The day before

1. Update the code and rebuild:
   ```sh
   cd /opt/larpity && git pull && cd server && cargo build --release
   ```
2. Load test from a laptop, with the fake phone still looping the test clip:
   ```sh
   cargo run --release --bin fake-viewers -- --url wss://larpity.yourdomain.ro/ws/viewer --count 25 --minutes 10
   ```
   Expect `video_fps` near 15, `gaps` 0 and `disconnects` 0. If not, the VPS
   or its bandwidth is the problem, not the phone: fix that first.
3. Send the viewer link and the QR code to the students (team issue #66).

## One hour before

1. Turn recording on, so the stream, the chat and the questions are saved:
   ```sh
   sed -i 's/^#LARPITY_RECORD_DIR=/LARPITY_RECORD_DIR=/' /etc/larpity.env
   systemctl restart larpity
   ```
   Recording uses about 0.8 GB per hour; check `df -h /var/lib/larpity` has room.
2. Stop the fake phone for good, so it can't take over from the real one:
   ```sh
   systemctl disable --now larpity-fake-phone
   ```
3. Start the real AI host and stop the stub (the AI issue #60 has the service):
   ```sh
   systemctl start larpity-ai
   systemctl disable --now larpity-stub-ai
   ```
   Keep this ready in another terminal as the fallback if the host dies:
   `systemctl start larpity-stub-ai`. The stub turns every message with a `?`
   into a question and beeps as its speech, so the button keeps working.
4. Open `https://larpity.yourdomain.ro/status` on a phone and keep it open.
   `ai` must be `true` and `recording` must be a path once the phone streams.
5. Strap the phone on the professional, open the phone link (the one with the
   token), tap Start. On `/status`, `phone` turns `true` and `stream` fills in.
6. Open the viewer page on one team phone: picture, sound after the tap, chat.

## During the session

Watch `/status` and the viewer page. What can go wrong, and what to do:

| What you see | What happened | What to do |
|---|---|---|
| Viewers see "Nu e în direct"; `phone` is `false` | the phone lost its network | nothing: it reconnects on its own with backoff (#20). If it stays down over a minute, the person with the professional checks the phone: screen on, network, battery |
| `phone` is `true` but viewers see a frozen picture | the phone's upload can't keep up | ask the professional to stand still a moment; if it keeps happening, the bitrate is too high for that network (device issue #59) |
| `ai` is `false` | the AI host died | `systemctl start larpity-stub-ai`; the last ranking and speech survive on the server, so the button still works. Check `journalctl -u larpity-ai -n 50` later |
| The button says "no question" | the queue is empty | normal at the start; the AI needs a few chat messages first |
| The question comes as text, no speech | TTS was slower than the press | the phone shows the text; if the speech arrives within 10 s it plays then |
| `/status` doesn't load | the server or Caddy is down | `systemctl status larpity caddy`; systemd restarts the server in 2 s, viewers reconnect in 1 to 10 s, the phone reconnects. Never start the fake phone during the session |
| Many viewers lag or drop | too many for the VPS | the numbers from the fake-viewers run say where the limit is; nothing to do live except ask students on Wi-Fi to stay on it |
| A rude message in the chat | there is no moderation (#53) | the AI ignores it for the ranking; talk to the class |

Logs, if needed: `journalctl -u larpity -f`.

## After the session

1. Stop recording and bring the fake phone back for the next test:
   ```sh
   sed -i 's/^LARPITY_RECORD_DIR=/#LARPITY_RECORD_DIR=/' /etc/larpity.env
   systemctl restart larpity
   systemctl enable --now larpity-fake-phone
   ```
2. Copy the recordings off the VPS:
   ```sh
   scp -r root@larpity.yourdomain.ro:/var/lib/larpity/recordings/ ./pilot-recordings/
   ```
   `larpity-<time>.h264` and `.pcm` are the stream (turn them into an MP4 with
   the command in `server/README.md`), `chat-<time>.jsonl` is every chat
   message and `questions-<time>.jsonl` every ranking and pick, for the
   write-up. The AI host's own log (#62) has the decisions behind the ranking.
3. Send the survey link to the students (#64) if the viewer page didn't (#70).
4. Delete the recordings from the VPS once they are copied, as agreed in the
   privacy page (#63).
