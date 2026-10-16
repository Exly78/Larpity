# ai/: the AI host

The AI host reads the students' chat, keeps a short list of the questions worth
asking, and makes the Romanian speech the professional hears when they press the
button. It is a separate program that talks to the server over `/ws/ai` (see the
AI section of [`PROTOCOL.md`](../PROTOCOL.md)). Owner: Ski.

Nothing here runs yet. This folder holds the decisions and the prompt; the code
comes with #30, once its language is picked. Until then,
`server/src/bin/stub-ai.rs` stands in for it.

## Providers (decided in #28)

Everything goes through **OpenRouter**, like the rest of our AI so far.

| Job | Model | Why |
|---|---|---|
| Ranking chat questions (#29) | Gemini 3.5 Flash Lite | Cheap, fast, good Romanian |
| Rougher questions | Gemini 3.8 Flash | Better reasoning when Flash Lite isn't enough |
| Escalation | Gemini 4 Argon | Replaces Sonnet 5.5 in our catalogue: better Romanian, and takes raw audio (e.g. explaining sounds from the job) |
| Speech (#31) | `google/gemini-3.8-flash-lite-tts` | Speaks Romanian with correct grammar, several voices, about 2-3 s for a 10-word sentence, inexpensive |

The OpenRouter API key is a secret: read it from an environment variable and
never commit it.

## How ranking works (#29)

The host does **not** re-rank the whole chat each time. For every new chat
message it makes one small decision against the questions already on the list:

| Decision | When | What happens to the list |
|---|---|---|
| `new` | A real question nobody has asked yet | Added at the end, with `count` 1, rewritten as one short spoken sentence |
| `merge` | Same meaning as a question already on the list, however it is worded | That question's `count` goes up by 1 and the chat id is added to its `chat_ids` |
| `ignore` | Greetings, thanks, reactions, spam, links, insults, gibberish, or a question already answered | Nothing changes |

Then it sends the whole list as a `ranking` message, most asked first (ties stay
in the order they were first asked), at most 20 questions.

This keeps the flow steady:

- The list changes by at most one question per chat message, so viewers don't see
  it reshuffle and the top question doesn't jump around while the professional
  is about to press the button.
- Each call is small (the current list plus one message), so it stays fast and
  cheap however long the session runs.
- A question keeps its id from the moment it is added. That matters because
  speech is made per id (#31), and the server never lists an id again once it
  has been picked.

The host's own code, not the model, assigns ids, counts, and the order, and
drops questions the server reports as `question_picked`. The model only decides.

### Three ways to make the decision

Any of the three works; they differ in how they get there.

| Option | How it decides | Accuracy | Delay | Cost |
|---|---|---|---|---|
| **Gemini 3.5 Flash Lite** with [`prompts/decide.md`](prompts/decide.md) | Reads the list and the message and answers `new`, `merge` or `ignore`, and writes the spoken sentence for `new` | Highest | Medium | Medium |
| **Reranker model** | Scores how close the new message is to each question on the list; above a threshold is `merge`, below is `new` | Medium | Low | Lowest |
| **Decision model** (a small classifier) | Labels the message `new`, `merge` or `ignore` directly | Medium | Lowest | Low |

A reranker or classifier can't rewrite text, and a reranker can't spot spam on its
own. With either of those, send only the `new` questions to Flash Lite for the
spoken-sentence rewrite. That is a few calls per session instead of one per message.

**Start with Gemini.** It does all three jobs in one call and is the most
accurate. Switch only if testing shows it is too slow or too expensive for a
busy chat.

### The spoken sentence

Each question on the list is read out through the professional's earphones, so
its text must be:

- Romanian, with comma-below diacritics (`ș ț`, never `ş ţ`)
- one sentence, ending in `?`, about 15 words at most (about 3-4 s of speech)
- spoken to the professional ("Ce ai studiat?"), with typos fixed and no
  nicknames, greetings or filler
- the question only, never an answer

## Testing on a fake chat log

[`tests/fake-chat.jsonl`](tests/fake-chat.jsonl) is a session as the server
would send it to the AI host: one message per line, exactly as in `PROTOCOL.md`.
It has repeats worded differently, near-misses that must *not* be merged,
spam, greetings, an insult, and a question asked again after it was answered.
[`tests/fake-chat.expected.json`](tests/fake-chat.expected.json) gives the
decision each message should get, and the final list.

To check a model or prompt, feed the messages in order: for each `chat`, send the
prompt with the current list and the message, apply the decision, and compare.
For each `question_picked`, remove that question from the list and add it to the
answered list. The decisions and merge targets must match exactly. The wording
of each `new` question can differ from the expected one, as long as it follows
the rules above.

This is done by hand for now (e.g. in the OpenRouter playground); #30 turns it
into a script once the host's language is picked.

## Still to do

- #30: the host itself: connect to `/ws/ai`, call the model per message, send
  `ranking`. Pick its language there.
- #31: make speech for each `new` question right away and send it as `speech`,
  so the button press plays it at once.
