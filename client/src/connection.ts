// The /ws/viewer socket, reconnecting with backoff (1 s, 2 s, 4 s, then every
// 10 s). The server replays config, frames, chat and questions on every connect,
// so `onOpen` is where the page resets to a clean state.

import type { AudioConfig } from "./audio.js";

/** One entry of the AI's ranking; `count` is how many chat messages asked it. */
export type Question = { id: string; text: string; count: number };

export type ServerMessage =
  | { type: "config"; video: { codec: string }; audio?: AudioConfig | null }
  | { type: "offline" }
  | { type: "joined"; nick: string; key: string }
  | { type: "chat"; id: number; nick: string; text: string; ts: number }
  | { type: "questions"; questions: Question[] }
  | { type: "question_picked"; id: string; text: string }
  | { type: "error"; code: string; message?: string };
// Types added later may also arrive; handlers ignore what they don't know.

export type Handlers = {
  onOpen(): void;
  onClose(): void;
  onMessage(msg: ServerMessage): void;
  onBinary?(data: ArrayBuffer): void;
};

const BACKOFF_MS = [1000, 2000, 4000, 10000];

export class Connection {
  private ws: WebSocket | null = null;
  private failures = 0;

  constructor(private handlers: Handlers) {
    this.open();
  }

  get isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Sends a JSON message. Returns false if the socket isn't open. */
  send(msg: object): boolean {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  private open(): void {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${scheme}://${location.host}/ws/viewer`);
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.onopen = () => {
      this.failures = 0;
      this.handlers.onOpen();
    };
    ws.onmessage = (e: MessageEvent) => {
      if (typeof e.data !== "string") {
        this.handlers.onBinary?.(e.data as ArrayBuffer);
        return;
      }
      let msg: unknown;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg && typeof msg === "object" && typeof (msg as { type?: unknown }).type === "string") {
        this.handlers.onMessage(msg as ServerMessage);
      }
    };
    // An error is always followed by close, so reconnecting is handled there.
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.handlers.onClose();
      const delay = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)];
      this.failures++;
      setTimeout(() => this.open(), delay);
    };
  }
}
