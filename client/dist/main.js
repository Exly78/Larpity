// The viewer page: chat (#25), video (#23), audio (#24), the "being answered"
// banner (#26) and the ranked questions (#90).
import { AudioPlayer, audioSupported } from "./audio.js";
import { Connection } from "./connection.js";
import { errorText, t, unknownError } from "./strings.js";
import { VideoPlayer, videoSupported } from "./video.js";
// PROTOCOL.md chat rules.
const MAX_NICK_CHARS = 24;
const MAX_TEXT_CHARS = 200;
// Keep the page light in a long session; the server replays only the last 50 anyway.
const MAX_CHAT_ITEMS = 200;
const NICK_KEY = "larpity.nick";
// The AI ranks at most 20 (PROTOCOL.md); the closed list shows the top few so the chat keeps its room.
const MAX_QUESTIONS = 20;
const QUESTIONS_SHOWN_CLOSED = 3;
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
const banner = $("banner");
const bannerText = $("banner-text");
const questionsBox = $("questions");
const questionsHead = $("questions-head");
const questionsList = $("questions-list");
const questionsMore = $("questions-more");
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
 * The key from the last `joined`, sent with every join. After a network switch
 * the server still holds our dead connection, and the key makes it free "Ana"
 * for us instead of answering "Ana2". In memory only: a reload closes the old
 * socket properly, so there is nothing to take back.
 */
let joinKey = null;
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
/** True between `config` and `offline`. */
let isLive = false;
/** The text of the last `question_picked`, or null if none has come since connecting. */
let pickedText = null;
/** The last `questions` list, best first; empty until one comes after connecting. */
let questions = [];
/** True while the student has the full list open. */
let questionsOpen = false;
// --- Rendering ---
function setStatus(text, live) {
    const status = $("status");
    status.textContent = text;
    status.classList.toggle("live", live);
}
function setStageMsg(text) {
    $("stage-msg").textContent = text;
}
function setLive(live) {
    isLive = live;
    renderBanner();
    renderQuestions();
    setStatus(live ? t.live : t.offline, live);
    if (!videoSupported())
        return setStageMsg(t.videoUnsupported);
    setStageMsg(live ? t.loadingVideo : t.waitingForStream);
}
function isChatAtBottom() {
    return chatList.scrollHeight - chatList.scrollTop - chatList.clientHeight < 40;
}
/**
 * Runs a change to the boxes above the chat. Growing, they shrink the chat without
 * scrolling it, which would hide the newest messages and stop addChat following
 * new ones, so a reader who was at the bottom is put back there.
 */
function keepChatAtBottom(change) {
    const atBottom = isChatAtBottom();
    change();
    if (atBottom)
        chatList.scrollTop = chatList.scrollHeight;
}
/**
 * Shows the question the professional is answering. Only while live: the server
 * keeps the last pick after the phone leaves, but then nobody is answering it.
 */
function renderBanner() {
    keepChatAtBottom(() => {
        banner.hidden = !isLive || pickedText === null;
        bannerText.textContent = pickedText ?? "";
    });
}
/**
 * Shows the ranked questions with their counts. Only while live, like the banner:
 * the server keeps its ranking while the phone is away but doesn't send it again
 * when the phone comes back, so the list is hidden rather than thrown away.
 */
function renderQuestions() {
    keepChatAtBottom(fillQuestions);
}
function fillQuestions() {
    // An empty list (after the last pick, or on reconnect) comes back closed.
    if (questions.length === 0)
        questionsOpen = false;
    questionsBox.hidden = !isLive || questions.length === 0;
    questionsBox.classList.toggle("open", questionsOpen);
    questionsHead.setAttribute("aria-expanded", String(questionsOpen));
    const more = questions.length > QUESTIONS_SHOWN_CLOSED;
    questionsMore.textContent = questionsOpen
        ? t.showFewerQuestions
        : more
            ? t.showAllQuestions(questions.length)
            : "";
    const shown = questionsOpen ? questions : questions.slice(0, QUESTIONS_SHOWN_CLOSED);
    questionsList.replaceChildren(...shown.map((q) => {
        const li = document.createElement("li");
        const text = document.createElement("span");
        text.className = "q-text";
        text.textContent = q.text;
        li.append(text);
        if (q.count > 0) {
            const count = document.createElement("span");
            count.className = "q-count";
            count.textContent = `×${q.count}`;
            li.append(count);
        }
        return li;
    }));
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
    const atBottom = isChatAtBottom();
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
            video?.configure(msg.video.codec);
            audio?.configure(msg.audio);
            break;
        case "offline":
            setLive(false);
            video?.reset();
            audio?.reset();
            break;
        case "joined":
            myNick = msg.nick;
            joinKey = msg.key;
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
        case "questions":
            questions = Array.isArray(msg.questions) ? msg.questions.slice(0, MAX_QUESTIONS) : [];
            renderQuestions();
            break;
        case "question_picked":
            pickedText = msg.text;
            // A new question starts clamped, even if the last one was opened up.
            banner.classList.remove("open");
            renderBanner();
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
/** Asks for a nickname. Returns false if the socket isn't open. */
function sendJoin(nick) {
    // The server ignores a key from this same connection, so it's safe on a name change too.
    return conn.send(joinKey ? { type: "join", nick, key: joinKey } : { type: "join", nick });
}
// --- Video ---
const video = videoSupported()
    ? new VideoPlayer($("video"), () => setStageMsg(""), () => setStageMsg(t.videoError))
    : null;
// --- Audio ---
// Works even where video doesn't, so those students can still listen.
const audio = audioSupported() ? new AudioPlayer($("listen-btn")) : null;
const conn = new Connection({
    onOpen() {
        // The server replays everything, so start clean.
        clearChat();
        pickedText = null;
        questions = [];
        setLive(false);
        video?.reset();
        audio?.reset();
        // Don't rejoin over a nickname the student is typing after "Schimbă".
        const choosingNick = myNick === null && !rejoining && !nickInBox;
        myNick = null;
        rejoining = wantedNick !== null && !choosingNick;
        renderComposer();
        if (rejoining && wantedNick)
            sendJoin(wantedNick);
    },
    onClose() {
        setStatus(t.connecting, false);
    },
    onMessage,
    onBinary(data) {
        video?.push(data);
        audio?.push(data);
    },
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
        if (!sendJoin(text)) {
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
// Long questions are cut to three lines so the chat keeps its room; a tap shows all of it.
banner.addEventListener("click", () => keepChatAtBottom(() => banner.classList.toggle("open")));
// The whole box is the tap target, not just its heading; the heading is a button for keyboards.
questionsBox.addEventListener("click", () => {
    questionsOpen = !questionsOpen;
    renderQuestions();
});
$("listen-btn").textContent = t.tapToListen;
$("banner-label").textContent = t.beingAnswered;
$("questions-title").textContent = t.questionsTitle;
$("me-label").textContent = t.writingAs;
$("change-nick").textContent = t.changeNick;
setStatus(t.connecting, false);
setStageMsg(videoSupported() ? t.waitingForStream : t.videoUnsupported);
clearChat();
renderComposer();
