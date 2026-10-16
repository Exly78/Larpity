// Keeps the phone's WebSocket to the server open across network drops and
// switches between Wi-Fi and mobile data (#20). Plain ES module, no packages.
//
//   import { PhoneSocket, phoneSocketUrl } from "./reconnect.js";
//   const socket = new PhoneSocket(phoneSocketUrl(), {
//     onOpen() { socket.send(JSON.stringify(config)); needKeyframe = true; },
//     onMessage(data) { ... },          // string (JSON) or ArrayBuffer (speech)
//     onStatus(status) { ... },         // see STATUS below
//   });
//
// Every new connection calls onOpen, so that is where the page sends `config`
// and makes its next encoded frame a keyframe, as PROTOCOL.md asks after a
// reconnect. The camera and encoder keep running while the socket is down;
// send() returns false then, and the page just drops those frames.
//
// A dead connection doesn't always close: after a network switch the old
// socket can sit there for a minute, especially on iPhone. So besides retrying
// on close, it gives up on a socket that can't open, or whose outgoing data has
// stopped moving, and opens a new one. Close code 4000 means another phone took
// over: it stops for good and never reconnects.

/** What onStatus reports. "replaced" and "stopped" are final. */
export const STATUS = {
  connecting: "connecting", // opening a socket
  open: "open", // connected; onOpen has run
  waiting: "waiting", // lost the connection; trying again soon
  replaced: "replaced", // another phone took over (close code 4000)
  stopped: "stopped", // stop() was called
};

const CLOSE_REPLACED = 4000;

const DEFAULTS = {
  firstRetryMs: 250, // the first retry is quick: a network switch is usually over in a moment
  maxRetryMs: 5000, // then back off, doubling, up to this
  openTimeoutMs: 5000, // give up on a socket that hasn't opened by then
  stallMs: 5000, // give up on a socket whose queued data hasn't moved for this long
  checkEveryMs: 1000, // how often to look for a stall
};

/** `/ws/phone` on the page's own origin, with the page's `?token=` passed on. */
export function phoneSocketUrl(loc = globalThis.location) {
  const protocol = loc.protocol === "https:" ? "wss:" : "ws:";
  const token = new URLSearchParams(loc.search).get("token") ?? "";
  return `${protocol}//${loc.host}/ws/phone?token=${encodeURIComponent(token)}`;
}

export class PhoneSocket {
  constructor(url, { onOpen = () => {}, onMessage = () => {}, onStatus = () => {}, ...timing } = {}) {
    this.url = url;
    this.handlers = { onOpen, onMessage, onStatus };
    this.timing = { ...DEFAULTS, ...timing };
    this.ws = null;
    this.status = null;
    this.failures = 0; // connections lost in a row, for the backoff
    this.retryTimer = null;
    this.openTimer = null;
    this.checkTimer = null;

    // Try again at once when the network comes back or the page is shown again
    // (iPhone suspends sockets while the screen is off), instead of waiting out
    // the backoff.
    this.retryNow = () => {
      if (this.status === STATUS.waiting) this.connect();
    };
    this.onVisibility = () => {
      if (globalThis.document?.visibilityState === "visible") this.retryNow();
    };
    globalThis.addEventListener?.("online", this.retryNow);
    globalThis.document?.addEventListener("visibilitychange", this.onVisibility);

    this.connect();
  }

  /** True while connected. */
  get isOpen() {
    return this.status === STATUS.open;
  }

  /** Bytes queued and not yet sent; the page can skip frames while this grows. */
  get bufferedAmount() {
    return this.ws?.bufferedAmount ?? 0;
  }

  /** Sends a string or binary frame. Returns false, sending nothing, while not connected. */
  send(data) {
    if (!this.isOpen) return false;
    this.ws.send(data);
    this.sentSinceCheck += data.byteLength ?? data.length;
    return true;
  }

  /** Closes the connection for good. */
  stop() {
    this.finish(STATUS.stopped);
  }

  connect() {
    clearTimeout(this.retryTimer);
    this.setStatus(STATUS.connecting);
    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    this.openTimer = setTimeout(() => this.lost(ws), this.timing.openTimeoutMs);

    ws.onopen = () => {
      if (ws !== this.ws) return;
      clearTimeout(this.openTimer);
      this.failures = 0;
      this.startStallCheck();
      this.setStatus(STATUS.open);
      this.handlers.onOpen();
    };
    ws.onmessage = (event) => {
      if (ws === this.ws) this.handlers.onMessage(event.data);
    };
    ws.onclose = (event) => {
      if (ws !== this.ws) return;
      if (event.code === CLOSE_REPLACED) this.finish(STATUS.replaced);
      else this.lost(ws);
    };
    // An error is always followed by a close, which handles it.
    ws.onerror = () => {};
  }

  // Opening a socket also lets the server replace a stale one we gave up on.
  // That one gets close code 4000, which is ignored here because it is no
  // longer this.ws.
  lost(ws) {
    if (ws !== this.ws) return;
    this.drop();
    const delay = Math.min(this.timing.maxRetryMs, this.timing.firstRetryMs * 2 ** this.failures);
    this.failures++;
    this.setStatus(STATUS.waiting);
    this.retryTimer = setTimeout(() => this.connect(), delay);
  }

  finish(status) {
    if (this.status === STATUS.replaced || this.status === STATUS.stopped) return;
    this.drop();
    clearTimeout(this.retryTimer);
    globalThis.removeEventListener?.("online", this.retryNow);
    globalThis.document?.removeEventListener("visibilitychange", this.onVisibility);
    this.setStatus(status);
  }

  drop() {
    clearTimeout(this.openTimer);
    clearInterval(this.checkTimer);
    const ws = this.ws;
    this.ws = null;
    if (ws && ws.readyState !== WebSocket.CLOSED) ws.close();
  }

  // The data has moved if less is queued now than was queued last time plus
  // what we sent since. Nothing queued is never a stall.
  startStallCheck() {
    const ws = this.ws;
    let queued = 0;
    let movedAt = Date.now();
    this.sentSinceCheck = 0;
    this.checkTimer = setInterval(() => {
      const now = ws.bufferedAmount;
      if (now === 0 || now < queued + this.sentSinceCheck) movedAt = Date.now();
      queued = now;
      this.sentSinceCheck = 0;
      if (Date.now() - movedAt > this.timing.stallMs) this.lost(ws);
    }, this.timing.checkEveryMs);
  }

  setStatus(status) {
    if (status === this.status) return;
    this.status = status;
    this.handlers.onStatus(status);
  }
}
