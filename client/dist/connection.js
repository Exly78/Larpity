// The /ws/viewer socket, reconnecting with backoff (1 s, 2 s, 4 s, then every
// 10 s). The server replays config, frames, chat and questions on every connect,
// so `onOpen` is where the page resets to a clean state.
const BACKOFF_MS = [1000, 2000, 4000, 10000];
export class Connection {
    constructor(handlers) {
        this.handlers = handlers;
        this.ws = null;
        this.failures = 0;
        this.open();
    }
    get isOpen() {
        return this.ws?.readyState === WebSocket.OPEN;
    }
    /** Sends a JSON message. Returns false if the socket isn't open. */
    send(msg) {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN)
            return false;
        this.ws.send(JSON.stringify(msg));
        return true;
    }
    open() {
        const scheme = location.protocol === "https:" ? "wss" : "ws";
        const ws = new WebSocket(`${scheme}://${location.host}/ws/viewer`);
        ws.binaryType = "arraybuffer";
        this.ws = ws;
        ws.onopen = () => {
            this.failures = 0;
            this.handlers.onOpen();
        };
        ws.onmessage = (e) => {
            if (typeof e.data !== "string") {
                this.handlers.onBinary?.(e.data);
                return;
            }
            let msg;
            try {
                msg = JSON.parse(e.data);
            }
            catch {
                return;
            }
            if (msg && typeof msg === "object" && typeof msg.type === "string") {
                this.handlers.onMessage(msg);
            }
        };
        // An error is always followed by close, so reconnecting is handled there.
        ws.onclose = () => {
            if (this.ws !== ws)
                return;
            this.ws = null;
            this.handlers.onClose();
            const delay = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)];
            this.failures++;
            setTimeout(() => this.open(), delay);
        };
    }
}
