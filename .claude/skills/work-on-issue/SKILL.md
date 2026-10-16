---
name: work-on-issue
description: Work on a Larpity GitHub issue from start to finish: read it, check what it depends on, branch, build, verify and write the PR description. Use when asked to do, start, pick up or work on an issue (e.g. "do #23", "pick up the next client task").
argument-hint: "[issue number]"
---

# Work on an issue

## 1. Read it

Read the issue and its comments: the GitHub MCP `issue_read` tool, or
`gh issue view <N> --comments`. The `[AREA]` prefix in the title names the
folder: `[SERVER]` is `server/`, `[CLIENT]` is `client/`, `[DEVICE]` and
`[AUDIO]` are `firmware/`, and `[AI]` is the AI host. `[TEAM]` issues are people's
work, not code, so tell the user that instead of starting.

## 2. Check what it depends on

"Needs X" in a title, or another issue named in it, is a dependency. Check whether
those issues are closed or their work is merged on `main`.

- If a dependency isn't done, stop and tell the user what is missing. Then offer
  what can be built now against a stand-in (`fake-phone` stands in for the phone,
  `stub-ai` for the AI host), and note in the PR what still has to be swapped in.
- Never build against your guess of an unwritten interface. If `PROTOCOL.md`
  doesn't define what you need, that is a protocol change: use the
  `change-protocol` skill.

## 3. Plan

Read the folder's CLAUDE.md and the parts of `PROTOCOL.md` the issue touches. Then
write down:

- the files you will change
- what is out of scope, even if it's tempting
- how you will check it works: which commands, and what output you expect

For anything bigger than a small fix, show the user this plan before writing code.

## 4. Branch

```sh
git fetch origin main
git switch -c <area>-<issue>-<short-name> origin/main   # e.g. client-23-live-stream
```

## 5. Build it

- Make small commits, each with an imperative summary line and a body saying why.
- End the commit that finishes the issue with `Closes #N`; use `Refs #N` when
  something is left (see step 7).
- Stay in the issue's folder. Write down any change needed in someone else's folder for the PR.

## 6. Verify

- Run the folder's checks from its CLAUDE.md. For `server/`: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- If the change touches the stream, chat, questions or the button, run the whole
  loop with the `run-stack` skill and look at the result.
- Some checks need hardware you don't have: a real Android phone or iPhone, a
  30-minute stream, mobile data. Write those down as a checklist for a person.
  Don't claim they pass.

## 7. Report, and open the PR if asked

Open a PR only when the user asks. Use this shape for the description, and for
your summary either way:

```markdown
<one or two sentences: what this does and why>

Closes #N    (or: Refs #N, if anything below is still open)

## What changed
- ...

## How it was tested
- the commands you ran and what they showed

## Needs a person
- [ ] Test on Android Chrome
- [ ] Test on iPhone Safari
- ...

## Needs changes elsewhere
- `firmware/` (@SKIMOSK): ...
```

Use `Closes` only when everything in the issue title is done, including the
"Needs a person" items.
