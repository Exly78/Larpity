// Live video (#23): decodes the H.264 frames the server relays and draws them
// on the stage canvas. PROTOCOL.md "Binary frames" and "Video (kind 1)".
const HEADER_BYTES = 10;
const KIND_VIDEO = 1;
// About 2 s at 15 fps. If the decoder falls this far behind (slow phone),
// skip to the next keyframe and start from there to stay live.
const MAX_QUEUE = 30;
export function videoSupported() {
    return typeof VideoDecoder !== "undefined";
}
export class VideoPlayer {
    /** `onFrame` runs after every drawn frame; `onError` when decoding fails for good. */
    constructor(canvas, onFrame, onError) {
        this.canvas = canvas;
        this.onFrame = onFrame;
        this.onError = onError;
        this.decoder = null;
        this.codec = null;
        /** Drop frames until a keyframe: after (re)configuring or falling behind. */
        this.needKey = true;
        /** A decoder error gets one fresh decoder; a second error in a row gives up. */
        this.retried = false;
        const ctx = canvas.getContext("2d");
        if (!ctx)
            throw new Error("No 2D canvas");
        this.ctx = ctx;
    }
    /** From the `config` message: start a fresh decoder for this codec. */
    configure(codec) {
        this.codec = codec;
        this.retried = false;
        this.restart();
    }
    /** On `offline` or a new connection: stop, blank the picture, wait for `config`. */
    reset() {
        this.codec = null;
        this.close();
        this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    }
    /** A binary message from the socket. Ignores anything that isn't video. */
    push(data) {
        if (data.byteLength <= HEADER_BYTES)
            return;
        const view = new DataView(data);
        if (view.getUint8(0) !== KIND_VIDEO)
            return;
        const key = (view.getUint8(1) & 1) === 1;
        // After a decode error the browser closes the decoder at once but calls the
        // error callback later; decoding in between would throw.
        if (this.decoder?.state !== "configured")
            return;
        if (this.decoder.decodeQueueSize > MAX_QUEUE) {
            if (!key) {
                this.needKey = true;
                return;
            }
            // Behind: throw the backlog away and start again from this keyframe.
            this.restart();
        }
        if (this.needKey && !key)
            return;
        this.needKey = false;
        this.decoder?.decode(new EncodedVideoChunk({
            type: key ? "key" : "delta",
            timestamp: Number(view.getBigUint64(2)),
            data: new Uint8Array(data, HEADER_BYTES),
        }));
    }
    restart() {
        this.close();
        this.needKey = true;
        if (this.codec === null)
            return;
        const decoder = new VideoDecoder({
            output: (frame) => this.draw(frame),
            error: () => {
                // A decoder that errors is closed for good.
                if (this.decoder !== decoder)
                    return;
                this.decoder = null;
                if (this.retried)
                    return this.onError();
                this.retried = true;
                this.restart();
            },
        });
        // Annex B: SPS and PPS ride inside every keyframe, so no `description`.
        try {
            decoder.configure({ codec: this.codec, optimizeForLatency: true });
        }
        catch {
            // A malformed codec string throws here instead of calling `error`.
            decoder.close();
            return this.onError();
        }
        this.decoder = decoder;
    }
    draw(frame) {
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
    close() {
        const decoder = this.decoder;
        this.decoder = null;
        if (decoder && decoder.state !== "closed")
            decoder.close();
    }
}
