---
name: steward
description: Drive a Larpity pull request to green and mergeable. Use when a PR's CI fails, it has a merge conflict, or reviewers left comments, and when watching a PR Claude opened.
---

# Drive a PR to green

Only Exly merges PRs in this repo (squash), never Claude. Claude's job is to get a PR
green and answer every review, then hand it back.

## CI is red

The only workflow is `.github/workflows/server.yml`, which runs for changes under
`server/`. Reproduce it before changing anything:

```sh
cd server
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- Fix the cause and re-run all three before pushing.
- Never skip, `#[ignore]`, delete or loosen a test to get green.
- Never silence clippy with `#[allow(...)]` unless a comment says why the lint is
  wrong for that line.
- The tests use real sockets with timeouts (3 s to wait for a message, 300 ms of
  silence for "nothing arrives"). If a test fails only sometimes, find the race and
  wait for an event instead of sleeping. Don't make the sleeps longer.
- A failure that also happens on `main` isn't this PR's fault. Say so on the PR
  with the failing step, and fix it in its own PR if asked.

## Merge conflict

- Merge `origin/main` into the branch: `git fetch origin main && git merge origin/main`.
  Don't rebase or force-push a branch someone else may have checked out.
- For `server/Cargo.lock`, take `main`'s version, then run `cargo build` to update
  it. Never edit it by hand.
- For `PROTOCOL.md`, keep both sides' changes and check that the server code matches the merged text.

## Review comments

- The owner of a folder has the last word on that folder (see the root CLAUDE.md).
- Small, clear asks (renames, nits, an extra test): do them, push, and reply on the thread with the commit.
- Design questions, or anything that changes another owner's folder or
  `PROTOCOL.md`: reply with a proposal and wait for an answer.
- Disagreeing: reply with the reason and leave the thread open for a person to decide.
- Every thread gets a reply, and you resolve the ones you fixed.

## Done

When CI is green, nothing conflicts and every thread has an answer, say once that
the PR is waiting for review. Don't push again just to re-trigger anything.

Once `client/` or `firmware/` have tooling, add their checks to CI and to this skill.
