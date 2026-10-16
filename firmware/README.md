# firmware

The phone page the professional opens on the streaming phone. It captures the
camera and mic, encodes H.264 with WebCodecs, sends everything to the server
over `/ws/phone`, and has the on-screen "next question" button.

The server hosts this folder at `/phone/`, so the page is served over the same
HTTPS origin as the WebSocket. Open it as `https://<host>/phone/?token=<token>`
and pass the token on to the WebSocket URL.

Message format: [`../PROTOCOL.md`](../PROTOCOL.md).
