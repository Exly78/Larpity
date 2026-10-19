// Live audio (#24): plays the pcm16 frames the server relays with Web Audio.
// PROTOCOL.md "Binary frames" and "Audio (kind 2)".
const HEADER_BYTES = 10;
const KIND_AUDIO = 2;
// Play this far behind the newest frame, so a short network stall doesn't cut
// the sound. The picture runs this much ahead, which is hard to notice: the one
// talking is the professional, behind the camera.
const LEAD_S = 0.25;
// If more than this is queued (a burst after a stall), drop frames to stay live.
// Live sound can't catch up, so whatever is queued stays as delay behind the picture.
const MAX_AHEAD_S = 0.5;
// The sample rates every browser must accept for an AudioBuffer; outside them
// createBuffer throws on every frame.
const MIN_RATE = 8000;
const MAX_RATE = 96000;
export function audioSupported() {
    return typeof AudioContext !== "undefined";
}
export class AudioPlayer {
    /** The "tap to listen" button: shown while there is sound the browser won't play yet. */
    constructor(button) {
        this.button = button;
        this.ctx = null;
        /** From `config`; null while offline or when the stream has no audio we can play. */
        this.sampleRate = null;
        /**
         * When the next frame starts, on the `ctx.currentTime` clock. Never reset by
         * hand: frames already queued keep playing, and starting new ones earlier would
         * play both at once. Once it falls behind the clock, `push` starts afresh.
         */
        this.nextTime = 0;
        button.addEventListener("click", () => this.start());
        this.render();
    }
    /** From the `config` message. Only mono `pcm16` is sent today; anything else stays silent. */
    configure(audio) {
        const ok = audio?.codec === "pcm16" &&
            audio.channels === 1 &&
            audio.sample_rate >= MIN_RATE &&
            audio.sample_rate <= MAX_RATE;
        this.sampleRate = ok ? audio.sample_rate : null;
        this.render();
    }
    /** On `offline` or a new connection: nothing to play until the next `config`. */
    reset() {
        this.sampleRate = null;
        this.render();
    }
    /** A binary message from the socket. Ignores anything that isn't audio. */
    push(data) {
        const ctx = this.ctx;
        if (!ctx || ctx.state !== "running" || this.sampleRate === null)
            return;
        // At least one 16-bit sample, or createBuffer throws.
        if (data.byteLength < HEADER_BYTES + 2)
            return;
        const view = new DataView(data);
        if (view.getUint8(0) !== KIND_AUDIO)
            return;
        const now = ctx.currentTime;
        if (this.nextTime <= now)
            this.nextTime = now + LEAD_S; // first frame, or we ran dry
        else if (this.nextTime - now > MAX_AHEAD_S)
            return;
        const count = (data.byteLength - HEADER_BYTES) >> 1;
        const buffer = ctx.createBuffer(1, count, this.sampleRate);
        const samples = buffer.getChannelData(0);
        for (let i = 0; i < count; i++) {
            samples[i] = view.getInt16(HEADER_BYTES + 2 * i, true) / 32768;
        }
        const source = ctx.createBufferSource();
        source.buffer = buffer;
        source.connect(ctx.destination);
        source.start(this.nextTime);
        this.nextTime += buffer.duration;
    }
    /** The button's tap. iPhone only lets an AudioContext start inside a tap handler. */
    start() {
        if (!this.ctx) {
            // iPhone mutes Web Audio when the ringer switch is on silent, unless the page
            // asks for a "playback" audio session (newer Safari only).
            const session = navigator.audioSession;
            if (session)
                session.type = "playback";
            this.ctx = new AudioContext();
            // A call, the lock screen or the browser can pause it later; then the button comes back.
            this.ctx.addEventListener("statechange", () => this.render());
        }
        // Refused outside a tap; the button then stays up for another try.
        this.ctx.resume().catch(() => { });
        this.render();
    }
    render() {
        this.button.hidden = this.sampleRate === null || this.ctx?.state === "running";
    }
}
