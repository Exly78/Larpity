// Every piece of text students see, in one place so a teammate can proofread it.
// Romanian uses comma-below ș ț (U+0219, U+021B), never the cedilla forms.
export const t = {
    live: "În direct",
    offline: "Nu e în direct",
    waitingForStream: "Așteptăm transmisia…",
    tapToListen: "Atinge pentru sunet",
    beingAnswered: "Se răspunde acum",
    chatEmpty: "Încă nu sunt mesaje. Pune prima întrebare!",
    chatPlaceholder: "Scrie o întrebare…",
    send: "Trimite",
    // Chat (#25)
    nickPlaceholder: "Alege-ți un nume",
    join: "Intră",
    writingAs: "Scrii ca",
    changeNick: "Schimbă",
    connecting: "Ne conectăm…",
    // Ranked questions (#90)
    questionsTitle: "Întrebări în așteptare",
    showAllQuestions: (n) => `Vezi toate (${n})`,
    showFewerQuestions: "Mai puțin",
    // Video (#23)
    loadingVideo: "Se încarcă imaginea…",
    videoUnsupported: "Browserul tău nu poate reda video. Încearcă Chrome sau actualizează iPhone-ul.",
    videoError: "Imaginea nu poate fi redată pe acest telefon.",
};
// Romanian text for the server's error codes (PROTOCOL.md, "Error codes").
// The server's own `message` is English, for developers, so it is never shown.
export const errorText = {
    bad_message: "Ceva n-a mers. Încearcă din nou.",
    join_required: "Alege-ți un nume înainte să scrii.",
    bad_nick: "Numele trebuie să aibă între 1 și 24 de caractere.",
    bad_text: "Mesajul trebuie să aibă între 1 și 200 de caractere.",
    rate_limited: "Prea repede! Așteaptă o secundă.",
};
export const unknownError = "Ceva n-a mers. Încearcă din nou.";
