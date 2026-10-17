// Live video (#23): decodes the H.264 frames the server relays and draws them
// on the stage canvas. PROTOCOL.md "Binary frames" and "Video (kind 1)".

const HEADER_BYTES = 10;
const KIND_VIDEO = 1;
// About 2 s at 15 fps. If the decoder falls this far behind (slow phone),
// skip to the next keyframe and start from there to stay live.
const MAX_QUEUE = 30;

export function videoSupported(): boolean {
  return typeof VideoDecoder !== "undefined";
}

export class VideoPlayer {
  private decoder: VideoDecoder | null = null;
  private codec: string | null = null;
  /** Drop frames until a keyframe: after (re)configuring or falling behind. */
  private needKey = true;
  /** A decoder error gets one fresh decoder; a second error in a row gives up. */
  private retried = false;
  private ctx: CanvasRenderingContext2D;

  /** `onFrame` runs after every drawn frame; `onError` when decoding fails for good. */
  constructor(
    private canvas: HTMLCanvasElement,
    private onFrame: () => void,
    private onError: () => void,
  ) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("No 2D canvas");
    this.ctx = ctx;
  }

  /** From the `config` message: start a fresh decoder for this codec. */
  configure(codec: string): void {
    this.codec = codec;
    this.retried = false;
    this.restart();
  }

  /** On `offline` or a new connection: stop, blank the picture, wait for `config`. */
  reset(): void {
    this.codec = null;
    this.close();
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  /** A binary message from the socket. Ignores anything that isn't video. */
  push(data: ArrayBuffer): void {
    if (data.byteLength <= HEADER_BYTES) return;
    const view = new DataView(data);
    if (view.getUint8(0) !== KIND_VIDEO) return;
    const key = (view.getUint8(1) & 1) === 1;

    if (!this.decoder) return;
    if (this.decoder.decodeQueueSize > MAX_QUEUE) {
      if (!key) {
        this.needKey = true;
        return;
      }
      // Behind: throw the backlog away and start again from this keyframe.
      this.restart();
    }
    if (this.needKey && !key) return;
    this.needKey = false;

    this.decoder?.decode(
      new EncodedVideoChunk({
        type: key ? "key" : "delta",
        timestamp: Number(view.getBigUint64(2)),
        data: new Uint8Array(data, HEADER_BYTES),
      }),
    );
  }

  private restart(): void {
    this.close();
    this.needKey = true;
    if (this.codec === null) return;
    const decoder = new VideoDecoder({
      output: (frame) => this.draw(frame),
      error: () => {
        // A decoder that errors is closed for good.
        if (this.decoder !== decoder) return;
        this.decoder = null;
        if (this.retried) return this.onError();
        this.retried = true;
        this.restart();
      },
    });
    // Annex B: SPS and PPS ride inside every keyframe, so no `description`.
    decoder.configure({ codec: this.codec, optimizeForLatency: true });
    this.decoder = decoder;
  }

  private draw(frame: VideoFrame): void {
    const w = frame.displayWidth;
    const h = frame.displayHeight;
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    this.ctx.drawImage(frame, 0, 0, w, h);
    frame.close();
    this.retried = false;
    this.onFrame();
  }

  private close(): void {
    const decoder = this.decoder;
    this.decoder = null;
    if (decoder && decoder.state !== "closed") decoder.close();
  }
}
