---
name: change-protocol
description: Change the Larpity wire protocol safely. Use whenever a change adds, removes or alters a WebSocket endpoint, JSON message, field, error code or binary frame layout, i.e. anything PROTOCOL.md describes, or when a feature needs data the protocol doesn't carry yet.
---

# Change the protocol

`PROTOCOL.md` is the contract between four people's code: the server, the phone
page, the viewer page and the AI host. A change that only one side knows about
breaks a live demo.

## 1. Decide whether it breaks anyone

- **Additive** (a new message type, or a new optional field): safe. Every part
  ignores unknown fields and message types, so old code keeps working.
- **Breaking** (renaming or removing something, changing what a field means or its
  type, or changing the binary header): needs agreement first. Comment on the
  issue naming the affected owners and wait for them before building it.

Prefer the additive version whenever it does the job.

## 2. Update PROTOCOL.md first

- Add or change the example JSON in the right section (phone, viewer or AI host).
- Write down when it is sent and what the receiver should do.
- New error codes go in the error table.
- Keep the examples realistic: Romanian question text, real-looking ids.

## 3. Update the server

- Types in `server/src/protocol.rs`: incoming messages are `Deserialize` enums
  tagged by `type`, outgoing ones are variants of `ServerMessage`. Use
  `#[serde(default)]` for new optional incoming fields, and
  `skip_serializing_if = "Option::is_none"` for optional outgoing ones.
- Handlers in `server/src/ws.rs` and state in `server/src/hub.rs`.
- Add an integration test in `server/tests/` that checks the exact JSON on the wire.
- If the phone's or the AI host's side changed, update `fake-phone` or `stub-ai`
  to match, so the test tools never drift from the protocol.

## 4. Find everyone else who is affected

Search `client/`, `firmware/` and the AI host's code for the message `type` or
field name. Don't edit folders that aren't yours. List what each owner must change:

| Part | Owner |
|---|---|
| `client/` | @katzow, @PantedJeans71 |
| `firmware/` | @SKIMOSK, @Exly78 |
| AI host | @SKIMOSK |
| `server/`, `PROTOCOL.md` | @Exly78 |

## 5. Check and hand over

- `cd server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- Start the PR title with `Protocol:`. In the description, show the before and
  after JSON, and add a "Needs changes elsewhere" section that @-mentions each
  affected owner with what they need to do.
