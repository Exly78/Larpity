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

## Working with Claude Code

The repo carries instructions so Claude Code follows the team's rules:

- `CLAUDE.md` here and in each folder: the rules and commands Claude reads. Keep
  them short and true. If Claude keeps getting something wrong, add a line; if a
  line stops mattering, delete it.
- Skills in `.claude/skills/`. Claude uses them on its own when they fit, or you
  can run them yourself:
  - `/work-on-issue 23`: work on an issue from start to finish
  - `/change-protocol`: change `PROTOCOL.md` without breaking anyone
  - `/run-stack`: run the server, fake phone and stub AI host locally
  - `/steward`: get a PR green and answer its reviews
- `.claude/settings.json` lets Claude run cargo commands without asking, formats
  Rust files after every edit (needs `jq`), and blocks pushing to `main` and merging PRs.
- Personal notes go in `CLAUDE.local.md`, which git ignores.

When `client/` and `firmware/` pick their tooling, put the commands in their CLAUDE.md.
