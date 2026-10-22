// Tests reconnect.js against a running server, through a TCP proxy that can cut
// connections (a drop) or freeze them (a dead network after a switch: nothing
// gets through, but nothing closes either).
//
//   LARPITY_TOKEN=dev larpity-server &          # see the run-stack skill
//   node --test firmware/tests/reconnect.test.mjs   # Node 22+, no packages
//
// LARPITY_URL (default http://127.0.0.1:8080) and LARPITY_TOKEN (default dev)
// point it at another server.

import assert from "node:assert/strict";
import net from "node:net";
import { afterEach, test } from "node:test";
import { PhoneSocket, STATUS, phoneSocketUrl } from "../reconnect.js";

const SERVER = new URL(process.env.LARPITY_URL ?? "http://127.0.0.1:8080");
const TOKEN = process.env.LARPITY_TOKEN ?? "dev";
const FAST = { firstRetryMs: 100, maxRetryMs: 400, openTimeoutMs: 1000, stallMs: 1500, checkEveryMs: 250 };
const CONFIG = JSON.stringify({
  type: "config",
  video: { codec: "avc1.42E01F", width: 1280, height: 720, framerate: 15 },
  audio: null,
});

// Each test stops its phones, so one test's phone doesn't take over from the next's.
const cleanup = [];
afterEach(() => cleanup.splice(0).forEach((fn) => fn()));

/** A TCP proxy to the server. cut() closes every connection; freeze() stops them dead. */
async function startProxy(port = 0) {
  const pairs = new Set();
  const frozen = new Set();
  const server = net.createServer((client) => {
    const upstream = net.connect(Number(SERVER.port || 80), SERVER.hostname);
    const pair = { client, upstream };
    pairs.add(pair);
    client.pipe(upstream);
    upstream.pipe(client);
    const end = () => {
      client.destroy();
      upstream.destroy();
      pairs.delete(pair);
    };
    client.on("error", end).on("close", end);
    upstream.on("error", end).on("close", end);
  });
  await new Promise((resolve) => server.listen(port, "127.0.0.1", resolve));
  const proxy = {
    port: server.address().port,
    cut() {
      for (const { client } of pairs) client.destroy();
    },
    freeze() {
      for (const pair of pairs) {
        const { client, upstream } = pair;
        client.unpipe(upstream);
        upstream.unpipe(client);
        client.pause();
        upstream.pause();
        frozen.add(pair);
      }
      pairs.clear(); // left open on purpose; new connections still go through
    },
    close() {
      server.close();
      for (const { client, upstream } of [...pairs, ...frozen]) {
        client.destroy();
        upstream.destroy();
      }
    },
  };
  cleanup.push(() => proxy.close());
  return proxy;
}

function phoneUrl(port) {
  return `ws://127.0.0.1:${port}/ws/phone?token=${encodeURIComponent(TOKEN)}`;
}

/** A phone that sends config on every connection and records what happens. */
function startPhone(url, timing = FAST) {
  const phone = { opens: 0, statuses: [] };
  phone.socket = new PhoneSocket(url, {
    ...timing,
    onOpen() {
      phone.opens++;
      phone.socket.send(CONFIG);
    },
    onStatus(status) {
      phone.statuses.push(status);
    },
  });
  cleanup.push(() => phone.socket.stop());
  return phone;
}

/** A viewer that collects the server's JSON messages by type. */
async function startViewer() {
  const ws = new WebSocket(new URL("/ws/viewer", SERVER.href.replace(/^http/, "ws")));
  ws.binaryType = "arraybuffer";
  const viewer = { types: [] };
  ws.onmessage = (event) => {
    if (typeof event.data === "string") viewer.types.push(JSON.parse(event.data).type);
  };
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error(`no server at ${SERVER.href}; start it first (see the top of this file)`));
  });
  cleanup.push(() => ws.close());
  return viewer;
}

async function until(what, check, ms = 5000) {
  const start = Date.now();
  while (!check()) {
    if (Date.now() - start > ms) assert.fail(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return Date.now() - start;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** A 10-byte frame header plus a payload of zeros; the server doesn't decode video. */
function fakeVideo(bytes, keyframe) {
  const out = new Uint8Array(10 + bytes);
  const view = new DataView(out.buffer);
  view.setUint8(0, 1);
  view.setUint8(1, keyframe ? 1 : 0);
  view.setBigUint64(2, BigInt(Date.now() * 1000));
  return out;
}

test("builds the socket URL from the page's address and token", () => {
  assert.equal(
    phoneSocketUrl({ protocol: "https:", host: "larpity.example:443", search: "?token=a b" }),
    "wss://larpity.example:443/ws/phone?token=a%20b",
  );
  assert.equal(phoneSocketUrl({ protocol: "http:", host: "localhost:8080", search: "" }), "ws://localhost:8080/ws/phone?token=");
});

test("reconnects after a dropped connection and sends config again", async () => {
  const viewer = await startViewer();
  const proxy = await startProxy();
  const phone = startPhone(phoneUrl(proxy.port));
  await until("the first connection", () => phone.opens === 1);
  await until("viewers to get config", () => viewer.types.includes("config"));

  viewer.types.length = 0;
  proxy.cut();
  const took = await until("a second connection", () => phone.opens === 2);
  await until("viewers to get config again", () => viewer.types.includes("config"));
  assert.ok(took < 1000, `reconnected in ${took} ms`);
  assert.deepEqual(phone.statuses, ["connecting", "open", "waiting", "connecting", "open"]);
});

test("gives up on a frozen connection and opens a new one", async () => {
  const proxy = await startProxy();
  const phone = startPhone(phoneUrl(proxy.port));
  await until("the first connection", () => phone.opens === 1);

  // Stream like a phone does, so a dead link shows up as data that stops moving.
  let n = 0;
  const streaming = setInterval(() => phone.socket.send(fakeVideo(64 * 1024, n++ % 30 === 0)), 50);
  cleanup.push(() => clearInterval(streaming));
  await sleep(300);

  proxy.freeze();
  const took = await until("a new connection", () => phone.opens === 2, 10000);
  clearInterval(streaming);
  assert.ok(phone.socket.isOpen);
  // The stall only shows once the kernel's socket buffer is full, which takes
  // a few seconds of frames on top of stallMs; loopback buffers are large.
  assert.ok(took < FAST.stallMs + 6000, `noticed and reconnected in ${took} ms`);
});

test("keeps retrying with backoff until the server can be reached", async () => {
  // Find a free port, then leave nothing listening on it for a while.
  const probe = await startProxy();
  const port = probe.port;
  probe.close();

  const phone = startPhone(phoneUrl(port));
  await sleep(1500);
  const tries = phone.statuses.filter((s) => s === STATUS.connecting).length;
  assert.ok(tries >= 3 && tries <= 8, `${tries} tries in 1.5 s with backoff`);
  assert.equal(phone.opens, 0);

  await startProxy(port);
  await until("a connection once the server is back", () => phone.opens === 1);
});

test("stops for good when another phone takes over", async () => {
  const proxy = await startProxy();
  const first = startPhone(phoneUrl(proxy.port));
  await until("the first phone", () => first.opens === 1);

  const second = startPhone(phoneUrl(proxy.port));
  await until("the first phone to be replaced", () => first.socket.status === STATUS.replaced);
  await sleep(1500); // long enough for several retries, if it were retrying
  assert.equal(first.opens, 1);
  assert.equal(second.opens, 1);
  assert.ok(second.socket.isOpen);
  assert.equal(first.socket.send(CONFIG), false);
});
