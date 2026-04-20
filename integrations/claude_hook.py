#!/usr/bin/env python3
"""
Claude Code hook — reports lifecycle events to the AI Agent Dashboard widget.

Reads the hook payload JSON from stdin, derives a chat_id from the session's
cwd (or falls back to session_id), and POSTs the given status to the widget.

Configure in ~/.claude/settings.json:
    {
      "hooks": {
        "UserPromptSubmit": [{"hooks": [{"type": "command",
            "command": "python <repo>/integrations/claude_hook.py working"}]}],
        "Stop": [{"hooks": [{"type": "command",
            "command": "python <repo>/integrations/claude_hook.py done"}]}],
        "SessionEnd": [{"hooks": [{"type": "command",
            "command": "python <repo>/integrations/claude_hook.py clear"}]}],
        "SessionStart": [{"hooks": [{"type": "command",
            "command": "python <repo>/integrations/claude_hook.py idle"}]}]
      }
    }

Per-user settings (projects_root, benign_closers, server_port) live in the
widget's app-data config.json, resolved the same way Tauri's `app_data_dir()`
does so editing the file from the tray menu and from scripts reaches the
same bytes.
"""
import json
import os
import sys
import time
import urllib.request
from pathlib import Path

BUNDLE_IDENTIFIER = "com.anothersava.ai-agent-dashboard"
DEFAULT_PORT = 9077


def default_config_path() -> Path:
    """Mirror of Tauri's app_data_dir()/config.json."""
    if sys.platform.startswith("win"):
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME")
        base = Path(xdg) if xdg else Path.home() / ".config"
    return base / BUNDLE_IDENTIFIER / "config.json"


def load_config(config_path: Path) -> dict:
    with open(config_path) as f:
        loaded = json.load(f)
    if not isinstance(loaded, dict):
        raise ValueError(f"{config_path} must contain a JSON object")
    return loaded


def widget_url(config: dict) -> str:
    raw_port = config.get("server_port")
    try:
        port = int(raw_port) if raw_port is not None else DEFAULT_PORT
    except (TypeError, ValueError):
        port = DEFAULT_PORT
    return f"http://127.0.0.1:{port}/api/status"


def derive_chat_id(cwd, session_id: str, projects_root) -> str:
    if isinstance(cwd, str) and cwd.strip():
        normalized = cwd.replace("\\", "/").rstrip("/")
        if isinstance(projects_root, str) and projects_root.strip():
            root = projects_root.replace("\\", "/").rstrip("/")
            if normalized.lower().startswith(root.lower() + "/"):
                rel = normalized[len(root) + 1:]
                if rel:
                    return rel.translate(str.maketrans("/-_", "   "))
        return os.path.basename(normalized) or normalized[:20]
    return f"claude-{session_id[:8] or 'unknown'}"


def last_assistant_ends_with_question(transcript_path, benign_closers=()) -> bool:
    """Walk the transcript JSONL and return True if the latest assistant text block ends with '?'.

    Used to distinguish "truly done" (Stop / idle_prompt without a trailing question)
    from "awaiting user response" (Claude asked the user something and is blocked).

    `benign_closers` are conversational closers like "What's next?" that end with '?'
    but don't actually block — matched case-insensitively at the end of the text.
    """
    if not isinstance(transcript_path, str) or not transcript_path.strip():
        return False
    try:
        last_text = ""
        with open(transcript_path, "r", encoding="utf-8") as f:
            for line in f:
                try:
                    msg = json.loads(line).get("message", {}) or {}
                except Exception:
                    continue
                if msg.get("role") != "assistant":
                    continue
                content = msg.get("content", "")
                if isinstance(content, str) and content.strip():
                    last_text = content.strip()
                elif isinstance(content, list):
                    for block in content:
                        if isinstance(block, dict) and block.get("type") == "text":
                            text = block.get("text", "")
                            if isinstance(text, str) and text.strip():
                                last_text = text.strip()
        if not last_text.endswith("?"):
            return False
        lower = last_text.lower()
        return not any(
            lower.endswith(closer.lower())
            for closer in benign_closers
            if isinstance(closer, str) and closer
        )
    except OSError:
        return False


def _clean_prompt(text: str) -> str:
    for ch in ('\n', '\r', '\t', '\v', '\f'):
        text = text.replace(ch, ' ')
    while '  ' in text:
        text = text.replace('  ', ' ')
    return text.strip()


def _notification_label(payload: dict) -> str:
    notif_type = payload.get("notification_type", "")
    message = payload.get("message", "") or ""
    if notif_type == "permission_prompt":
        tool = message.rsplit("use ", 1)[-1] if "use " in message else "tool"
        return f"needs approval: {tool}"
    if notif_type == "plan_approval":
        return "plan approval"
    return message


def classify(arg: str, payload: dict, benign_closers=()) -> tuple[str, str | None]:
    """Map hook argv + payload to (status, label).

    label=None means "don't set the label on the wire" (widget preserves prior value).
    """
    transcript_path = payload.get("transcript_path")
    if arg == "working":
        prompt = payload.get("prompt")
        if isinstance(prompt, str) and prompt.strip():
            return "working", _clean_prompt(prompt)
        return "working", None
    if arg == "done":
        if last_assistant_ends_with_question(transcript_path, benign_closers):
            return "awaiting", "has a question"
        return "done", None
    if arg == "idle":
        notif_type = payload.get("notification_type")
        message = payload.get("message")
        if not notif_type and not (isinstance(message, str) and message.strip()):
            return "idle", None
        if notif_type == "idle_prompt":
            if last_assistant_ends_with_question(transcript_path, benign_closers):
                return "awaiting", "has a question"
            return "done", None
        label = _notification_label(payload)
        label = label.strip().splitlines()[0][:60] if isinstance(label, str) and label.strip() else None
        return "awaiting", label
    return arg, None


def build_body(arg: str, payload: dict, chat_id: str, benign_closers=()) -> dict:
    if arg == "clear":
        return {"action": "clear", "id": chat_id}
    status, label = classify(arg, payload, benign_closers)
    body = {
        "action": "set",
        "id": chat_id,
        "status": status,
        "source": "claude",
        "updated": int(time.time() * 1000),
    }
    transcript_path = payload.get("transcript_path")
    if isinstance(transcript_path, str) and transcript_path.strip():
        body["transcript_path"] = transcript_path
    if label:
        body["label"] = label
    return body


def post(url: str, body: dict) -> None:
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        urllib.request.urlopen(req, timeout=2)
    except Exception:
        pass  # widget may not be running — swallow


def main() -> None:
    if len(sys.argv) < 2:
        return
    arg = sys.argv[1]
    try:
        payload = json.load(sys.stdin)
    except Exception:
        payload = {}
    try:
        config = load_config(default_config_path())
    except (OSError, ValueError, json.JSONDecodeError):
        # No readable config (widget never ran, or malformed file) — fall
        # back to defaults so Claude hooks don't hard-fail.
        config = {}
    chat_id = derive_chat_id(
        payload.get("cwd"),
        payload.get("session_id") or "",
        config.get("projects_root"),
    )
    benign = config.get("benign_closers") or []
    body = build_body(arg, payload, chat_id, benign)
    post(widget_url(config), body)


if __name__ == "__main__":
    main()
