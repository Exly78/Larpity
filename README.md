# Larpity larp larp

A professional wears a phone and streams their workday live to students. Students
watch in the browser and ask questions in the chat; an AI host merges and ranks
the questions, and when the professional presses a button the top question is
read out to them in Romanian through their earphones.

## Layout

| Folder | What lives there | Owners |
|---|---|---|
| [`firmware/`](firmware/) | The phone page: camera, mic, H.264 encoding, the "next question" button | Ski, Exly |
| [`server/`](server/) | Rust server: relays the phone's stream to viewers, runs the chat, talks to the AI host | Exly |
| [`client/`](client/) | The viewer page students open on their phones | katzow, PantedJeans71 |

Every part talks to the others through the server, using the message format in
[`PROTOCOL.md`](PROTOCOL.md). Read that first.
