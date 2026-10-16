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
/** The nickname we asked for, re-sent on every reconnect. */
let wantedNick = loadNick();
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
    const joined = myNick !== null;
    input.placeholder = joined ? t.chatPlaceholder : t.nickPlaceholder;
    input.maxLength = joined ? MAX_TEXT_CHARS : MAX_NICK_CHARS;
    input.enterKeyHint = joined ? "send" : "go";
    sendBtn.textContent = joined ? t.send : t.join;
    $("me").hidden = !joined;
    $("me-nick").textContent = myNick ?? "";
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
            wantedNick = msg.nick;
            saveNick(msg.nick);
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
        myNick = null;
        renderComposer();
        if (wantedNick)
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
    if (myNick === null) {
        wantedNick = text;
        nickInBox = true;
        if (!conn.send({ type: "join", nick: text }))
            showError(t.connecting);
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
