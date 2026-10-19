# Larpity

A professional wears a phone and streams their workday live to students. Students
watch on their phones and ask questions in the chat; an AI host merges and ranks
the questions; the professional presses a button and hears the top question in
Romanian through earphones.

This is a competition entry with a pitch and a live pilot session coming up.
**Simple and reliable beats clever.** Pick the plainest approach that works on
real phones, and don't build for cases the pilot won't hit.

## Layout and owners

| Path | What | Owner |
|---|---|---|
| `PROTOCOL.md` | Message format between every part. The contract. | Exly |
| `server/` | Rust server: relay, chat, AI interface, recording; `fake-phone` and `stub-ai` test tools | Exly |
| `firmware/` | Phone page: camera, mic, H.264 encoding, the button (not built yet) | Ski, Exly |
| `client/` | Viewer page students open on their phones (not built yet) | katzow, PantedJeans71 |
| AI host | LLM ranking and Romanian TTS. Where it lives and its language are not decided (#28-#31) | Ski |

Work is tracked as GitHub issues on `Exly78/Larpity`, titled `[AREA] ...`. "Needs X"
in a title means the issue is blocked on X. Each folder has its own CLAUDE.md with
its commands and rules; it loads when you touch files there.

## Rules

- `PROTOCOL.md` is the contract between owners. Never change a message, field,
  error code or frame layout in code without changing `PROTOCOL.md` in the same
  PR. Use the `change-protocol` skill.
- Stay in the folder of the issue you're working on. If the work needs a change
  in someone else's folder, list it in the PR for that owner instead of making it.
- Text people see in the apps (buttons, banners, messages shown to students or the
  professional) is **Romanian**. Code, comments, docs, commits and PRs are **English**.
- Romanian text uses comma-below diacritics: `ș ț Ș Ț` (U+0219, U+021B), never
  the cedilla forms `ş ţ`.
- No new framework, service or large dependency without the folder owner agreeing
  in an issue.
- Everything must work on Android Chrome and iPhone Safari. The camera and
  WebCodecs only work over HTTPS (or `localhost`).
- Never commit secrets: tokens, API keys, `.env` files.

## Git and GitHub

- Never commit to or push `main`. Branch from an up-to-date `origin/main`, named
  `<area>-<issue>-<short-name>`, e.g. `client-23-live-stream`.
- Commits: an imperative summary line, then a body saying why. Reference the
  issue with `Refs #N`, never `Closes #N` / `Fixes #N`: those close the issue on
  merge, which moves it to Done and skips Exly's check.
- Open a PR only when asked. Only Exly accepts and merges PRs (squash). Never
  merge a PR and never close an issue.
- CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` in `server/` for every PR that touches it, and `npm run check`,
  `npm run build` and a check that the committed `dist/` is up to date in
  `client/` for every PR that touches that.

## Issue status

- **IMPORTANT:** When your work on an issue is finished (PR opened, or the work
  handed over), move the issue to **In review** on the project board, every time.
  If you can't change the board from your tools, tell the user to move it.
- **Only Exly moves an issue to Done** (and closes it), after checking the work.
  Never move an issue to Done or close it, even if the work looks complete or someone asks.

## Checking your work

Before saying something works, run the folder's checks (in its CLAUDE.md). For
anything that touches the stream, chat, questions or the button, also run the
whole stack with the `run-stack` skill. In your summary, say what you ran and what
you could not test, such as "not tried on an iPhone".

Personal preferences go in `CLAUDE.local.md` (gitignored), not here.
