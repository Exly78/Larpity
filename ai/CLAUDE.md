# ai/: the AI host

@../PROTOCOL.md

Reads the chat over `/ws/ai`, keeps the ranked question list, and makes the
Romanian speech. Owner: Ski. Issues #28-#31. `README.md` has the providers, how
ranking works, and how to test it.

**No code yet, and its language isn't chosen.** Pick it in #30; until then don't
add a runtime, framework or package file on your own. When it lands, put the
run and check commands here.

## Rules

- Models go through OpenRouter. Read the API key from an environment variable;
  never commit a key or a `.env` file.
- One decision per chat message (`new`, `merge`, `ignore`) against the current
  list, never a re-rank of the whole chat. The host's code, not the model,
  assigns ids, counts and the order.
- Ids are never reused, not even after a restart: start them with the host's
  start time, as `PROTOCOL.md` says.
- If you change `prompts/decide.md`, run it on `tests/fake-chat.jsonl` and check it
  still gives `tests/fake-chat.expected.json`. Change the expected file only if
  the expected behaviour itself changes, and say why in the PR.
