// The viewer page. Chat (#25) is live; video (#22/#23), audio (#24) and the
// "being answered" banner (#26) plug into the message handler below.
import { Connection } from "./connection.js";
import { errorText, t, unknownError } from "./strings.js";
// PROTOCOL.md chat rules.
const MAX_NICK_CHARS = 24;
const MAX_TEXT_CHARS = 200;
// Keep the page light in a long session; the server replays only the last 50 anyway.
const MAX_CHAT_ITEMS = 200;
const NICK_KEY = "larpity.nick";
function $(id) {
    const el = document.getElementById(id);
    if (!el)
        throw new Error(`Missing element: #${id}`);
    return el;
}
const chatList = $("chat");
const input = $("chat-input");
const sendBtn = $("send-btn");
const errorLine = $("chat-error");
// --- Saved nickname (so a reload or reconnect joins again without asking) ---
function loadNick() {
    try {
        return localStorage.getItem(NICK_KEY);
    }
    catch {
        return null;
    }
}
function saveNick(nick) {
    try {
        localStorage.setItem(NICK_KEY, nick);
    }
    catch {
        // Private mode or blocked storage: the nickname just isn't remembered.
    }
}
// --- State ---
/** The nickname the server confirmed with `joined`, or null before joining. */
let myNick = null;
/**
 * The nickname the student typed, re-sent on every reconnect. Never the server's
 * deduplicated "Ana2": asking for that would turn it into "Ana22" next time.
 */
let wantedNick = loadNick();
/**
 * True while a join with `wantedNick` is on its way and `joined` hasn't come back.
 * The box stays in chat mode and sends nothing, so a half-typed message can't go
 * out as a nickname. Starts true with a saved name: the first connect joins with it.
 */
let rejoining = wantedNick !== null;
/** The last message sent, put back in the box if the server rejects it. */
let lastSent = "";
/** True while the box holds a nickname the user submitted and we wait for `joined`. */
let nickInBox = false;
// --- Rendering ---
function setStatus(text, live) {
    const status = $("status");
    status.textContent = text;
    status.classList.toggle("live", live);
}
function setLive(live) {
    setStatus(live ? t.live : t.offline, live);
    $("stage-msg").textContent = live ? "" : t.waitingForStream;
}
function clearChat() {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = t.chatEmpty;
    chatList.replaceChildren(li);
}
function addChat(nick, text) {
    // Stick to the bottom only if the reader is already there, so scrolling up
    // to read older messages isn't interrupted.
    const atBottom = chatList.scrollHeight - chatList.scrollTop - chatList.clientHeight < 40;
    chatList.querySelector(".empty")?.remove();
    const li = document.createElement("li");
    if (nick === myNick)
        li.className = "mine";
    const nickEl = document.createElement("span");
    nickEl.className = "nick";
    nickEl.textContent = nick;
    li.append(nickEl, text);
    chatList.append(li);
    while (chatList.children.length > MAX_CHAT_ITEMS)
        chatList.firstElementChild?.remove();
    if (atBottom)
        chatList.scrollTop = chatList.scrollHeight;
}
let errorTimer;
function showError(text) {
    clearTimeout(errorTimer);
    errorLine.hidden = text === null;
    errorLine.textContent = text ?? "";
    if (text !== null)
        errorTimer = window.setTimeout(() => showError(null), 4000);
}
/** The box asks for a nickname until the server confirms one, then sends chat. */
function renderComposer() {
    const joined = myNick !== null || rejoining;
    input.placeholder = joined ? t.chatPlaceholder : t.nickPlaceholder;
    input.maxLength = joined ? MAX_TEXT_CHARS : MAX_NICK_CHARS;
    input.enterKeyHint = joined ? "send" : "go";
    sendBtn.textContent = joined ? t.send : t.join;
    $("me").hidden = !joined;
    $("me-nick").textContent = myNick ?? wantedNick ?? "";
}
// --- Server messages ---
function onMessage(msg) {
    switch (msg.type) {
        case "config":
            setLive(true);
            break;
        case "offline":
            setLive(false);
            break;
        case "joined":
            myNick = msg.nick;
            rejoining = false;
            // Keep asking for "Ana", not "Ana2": the server may still hold our old
            // connection after a network switch, and adds a digit to whatever we ask for.
            if (wantedNick)
                saveNick(wantedNick);
            // Clear the nickname out of the box, but not text typed while reconnecting.
            if (nickInBox)
                input.value = "";
            nickInBox = false;
            showError(null);
            renderComposer();
            break;
        case "chat":
            addChat(msg.nick, msg.text);
            break;
        case "error":
            if (msg.code === "join_required" || msg.code === "bad_nick") {
                myNick = null;
                rejoining = false;
                // The box now holds a name to fix, not one waiting for `joined`.
                nickInBox = false;
                // Don't retry a rejected nickname on the next reconnect.
                if (msg.code === "bad_nick")
                    wantedNick = loadNick();
                renderComposer();
            }
            if ((msg.code === "rate_limited" || msg.code === "bad_text") && input.value === "") {
                input.value = lastSent;
            }
            showError(errorText[msg.code] ?? unknownError);
            break;
    }
}
const conn = new Connection({
    onOpen() {
        // The server replays everything, so start clean.
        clearChat();
        setLive(false);
        // Don't rejoin over a nickname the student is typing after "Schimbă".
        const choosingNick = myNick === null && !rejoining && !nickInBox;
        myNick = null;
        rejoining = wantedNick !== null && !choosingNick;
        renderComposer();
        if (rejoining && wantedNick)
            conn.send({ type: "join", nick: wantedNick });
    },
    onClose() {
        setStatus(t.connecting, false);
    },
    onMessage,
});
// --- Input ---
$("composer").addEventListener("submit", (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (text === "")
        return;
    if (rejoining) {
        showError(t.connecting);
        return;
    }
    if (myNick === null) {
        // Only remember the name once it's sent, so a failed send can't become the
        // nickname we rejoin with.
        if (!conn.send({ type: "join", nick: text })) {
            showError(t.connecting);
            return;
        }
        wantedNick = text;
        nickInBox = true;
        return;
    }
    if (!conn.send({ type: "chat", text })) {
        showError(t.connecting);
        return;
    }
    // Not shown yet: it appears when the server sends it back to everyone.
    lastSent = text;
    input.value = "";
});
$("change-nick").addEventListener("click", () => {
    if (rejoining) {
        showError(t.connecting);
        return;
    }
    myNick = null;
    renderComposer();
    input.value = wantedNick ?? "";
    input.focus();
    input.select();
});
$("listen-btn").textContent = t.tapToListen;
$("banner-label").textContent = t.beingAnswered;
$("me-label").textContent = t.writingAs;
$("change-nick").textContent = t.changeNick;
setStatus(t.connecting, false);
$("stage-msg").textContent = t.waitingForStream;
clearChat();
renderComposer();
