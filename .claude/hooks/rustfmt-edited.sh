#!/bin/sh
# PostToolUse hook: after Claude edits or writes a .rs file, format it with
# rustfmt (picking up server/rustfmt.toml), so `cargo fmt --check` in CI never
# fails on Claude's edits. Exits quietly when jq or rustfmt isn't installed, and
# never blocks the edit: a syntax error will show up in cargo build anyway.
command -v jq >/dev/null 2>&1 || exit 0
command -v rustfmt >/dev/null 2>&1 || exit 0
file=$(jq -r '.tool_input.file_path // empty')
case "$file" in
  *.rs) rustfmt --edition 2024 "$file" >/dev/null 2>&1 ;;
esac
exit 0
