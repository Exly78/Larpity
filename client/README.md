# client

The viewer page students open. Plain HTML and TypeScript is enough. It connects
to `/ws/viewer`, decodes the H.264 stream with WebCodecs, plays audio with Web
Audio, and shows the chat and the "question being answered" banner.

The server hosts this folder at `/`, so `https://<host>/` opens `index.html`
from here.

Message format: [`../PROTOCOL.md`](../PROTOCOL.md).
