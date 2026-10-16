"""Runs prompts/decide.md on fake-chat.jsonl and checks it against fake-chat.expected.json.

    OPENROUTER_API_KEY=... python3 ai/tests/run_fake_chat.py [--model ID]

Plain Python 3, no packages. Each message is judged on its own: the model always
sees the list as it should be at that point (built from the expected decisions),
so one wrong answer doesn't throw off the rest. A "new" question's wording is
checked against the rules in ai/README.md, not compared word for word.
Exits 0 when every decision matches.
"""

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROMPT = HERE.parent / "prompts" / "decide.md"
DEFAULT_MODEL = "google/gemini-3.5-flash-lite"
DEFAULT_URL = "https://openrouter.ai/api/v1/chat/completions"
MAX_WORDS = 15


def load_prompt():
    """The system prompt and the user message template: the first two ```text blocks."""
    blocks = re.findall(r"```text\n(.*?)```", PROMPT.read_text(encoding="utf-8"), re.S)
    return blocks[0].strip(), blocks[1].strip()


def ask(args, key, system, user):
    body = json.dumps({
        "model": args.model,
        "temperature": 0,
        "response_format": {"type": "json_object"},
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
    }).encode()
    request = urllib.request.Request(args.url, body, {
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
    })
    with urllib.request.urlopen(request, timeout=60) as response:
        reply = json.load(response)
    text = reply["choices"][0]["message"]["content"].strip()
    # Some models wrap JSON in a code fence even in JSON mode.
    text = re.sub(r"^```(?:json)?\s*|\s*```$", "", text)
    return json.loads(text)


def text_problems(text):
    problems = []
    if not text.endswith("?"):
        problems.append("doesn't end with ?")
    if len(text.split()) > MAX_WORDS:
        problems.append(f"over {MAX_WORDS} words")
    if any(c in text for c in "şţŞŢ"):
        problems.append("cedilla ş/ţ instead of ș/ț")
    if len(text) > 300:
        problems.append("over 300 characters")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model", default=DEFAULT_MODEL, help=f"OpenRouter model id (default {DEFAULT_MODEL})")
    parser.add_argument("--url", default=DEFAULT_URL, help="chat completions endpoint")
    args = parser.parse_args()
    key = os.environ.get("OPENROUTER_API_KEY")
    if not key:
        sys.exit("set OPENROUTER_API_KEY")

    system, template = load_prompt()
    log = [json.loads(line) for line in (HERE / "fake-chat.jsonl").read_text(encoding="utf-8").splitlines()]
    expected = json.loads((HERE / "fake-chat.expected.json").read_text(encoding="utf-8"))["decisions"]

    questions, answered = [], []  # the list as it should be, from the expected decisions
    passed = failed = 0
    delays = []
    for message, want in zip(log, expected):
        if message["type"] == "question_picked":
            answered += [q["text"] for q in questions if q["id"] == message["id"]]
            questions = [q for q in questions if q["id"] != message["id"]]
            continue

        user = (template
                .replace("{questions}", json.dumps(questions, ensure_ascii=False))
                .replace("{answered}", json.dumps(answered, ensure_ascii=False))
                .replace("{message}", json.dumps(message["text"], ensure_ascii=False)))
        start = time.monotonic()
        try:
            got = ask(args, key, system, user)
        except (urllib.error.URLError, KeyError, IndexError, ValueError) as err:
            got = {"action": f"error: {err}"}
        delays.append(time.monotonic() - start)

        problems = []
        if got.get("action") != want["decision"]:
            problems.append(f"expected {want['decision']}")
        elif want["decision"] == "merge" and got.get("id") != want["id"]:
            problems.append(f"merged into {got.get('id')}, expected {want['id']}")
        elif want["decision"] == "new":
            problems += text_problems(str(got.get("text", "")))

        shown = got.get("action", "?")
        shown += f" {got['id']}" if "id" in got else ""
        shown += f" \"{got['text']}\"" if "text" in got else ""
        mark = "ok  " if not problems else "FAIL"
        print(f"{mark} #{message['id']:<3} {message['text']!r} -> {shown}"
              + (f"  ({'; '.join(problems)})" if problems else "")
              + f"  [{delays[-1]:.1f}s]")
        passed, failed = passed + (not problems), failed + bool(problems)

        # Move on from the expected decision, not the model's.
        if want["decision"] == "new":
            questions.append({"id": want["id"], "text": want["text"], "count": 1})
        elif want["decision"] == "merge":
            next(q for q in questions if q["id"] == want["id"])["count"] += 1
        questions.sort(key=lambda q: -q["count"])

    delays.sort()
    print(f"\n{passed}/{passed + failed} decisions right with {args.model}; "
          f"delay median {delays[len(delays) // 2]:.1f}s, worst {delays[-1]:.1f}s")
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
