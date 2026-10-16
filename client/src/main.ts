// Layout sketch for #21. Nothing connects to the server yet: the WebSocket,
// video (#22/#23), audio (#24), chat (#25) and banner (#26) replace the
// placeholder data below.
import { t } from "./strings.js";

type ChatMessage = { nick: string; text: string };

function $<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Missing element: #${id}`);
  return el as T;
}

function setLive(live: boolean): void {
  const status = $("status");
  status.textContent = live ? t.live : t.offline;
  status.classList.toggle("live", live);
  $("stage-msg").textContent = live ? "" : t.waitingForStream;
}

function showBanner(text: string | null): void {
  $("banner").hidden = text === null;
  $("banner-text").textContent = text ?? "";
}

function renderChat(messages: ChatMessage[]): void {
  const list = $<HTMLUListElement>("chat");
  if (messages.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = t.chatEmpty;
    list.replaceChildren(li);
    return;
  }
  list.replaceChildren(
    ...messages.map((m) => {
      const li = document.createElement("li");
      const nick = document.createElement("span");
      nick.className = "nick";
      nick.textContent = m.nick;
      li.append(nick, m.text);
      return li;
    }),
  );
  list.scrollTop = list.scrollHeight;
}

function fillStaticText(): void {
  $("listen-btn").textContent = t.tapToListen;
  $("banner-label").textContent = t.beingAnswered;
  $<HTMLInputElement>("chat-input").placeholder = t.chatPlaceholder;
  $("send-btn").textContent = t.send;
}

fillStaticText();
$<HTMLFormElement>("composer").addEventListener("submit", (e) => e.preventDefault());

// Placeholder state so the layout can be looked at. Open with ?empty to see
// the offline, no-messages state instead.
if (new URLSearchParams(location.search).has("empty")) {
  setLive(false);
  showBanner(null);
  renderChat([]);
} else {
  setLive(true);
  showBanner("Cât durează o operație?");
  renderChat([
    { nick: "Ana", text: "Ce ai studiat?" },
    { nick: "Mihai", text: "Cât durează o operație?" },
    { nick: "Ioana", text: "Îți place ce faci?" },
    { nick: "Andrei", text: "Câte ore lucrezi pe zi?" },
  ]);
}
