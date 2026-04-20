---
layout: default
title: HTTP API
---

[Home](..) | [Claude Code](claude-code) | [HTTP API](http-api) | [Development](development)

---

The widget listens on `http://127.0.0.1:9077` (default) for status updates. Any tool that can POST JSON — curl, Python, a CI job, a background agent — can report status without any library dependency.

### Endpoint

`POST /api/status` with `Content-Type: application/json`. Returns `204 No Content` on success, `403` if the `Origin` header is a real web origin (blocks browser XHR), `400` on malformed JSON.

### Actions

The body's `action` field selects the operation. All other fields depend on the action.

**`set`** — create or update a row.

```json
{
  "action": "set",
  "id": "my-task",
  "status": "working",
  "label": "Building the report",
  "source": "script",
  "inputTokens": 42000,
  "model": "claude-opus-4-7",
  "transcript_path": "/abs/path/to/transcript.jsonl"
}
```

- `id` — session identifier; subsequent `set`s with the same id update the row.
- `status` — one of `working`, `awaiting`, `idle`, `done`, `error`.
- `label` — optional. Omitting it preserves whatever label the row already had, which is useful when you want to change only status.
- `source` — free-form, defaults to `claude-code`. Preserved for future multi-agent styling.
- `inputTokens` / `model` — optional token count and model name. The token count is colored by `%` of the model's context window (looked up in `config.json`).
- `transcript_path` — optional. When provided, the widget starts tailing that JSONL and pulls model / token info live from assistant turns.

**`clear`** — remove a row.

```json
{"action": "clear", "id": "my-task"}
```

Stops any transcript watcher attached to that session.

**`config`** — update widget configuration.

```json
{"action": "config", "always_on_top": false}
```

Any keys in the body (other than `action`) are merged into the live config and persisted to `config.json`. The widget applies the change immediately — `always_on_top` flips, the tray check marks sync. Port changes persist but don't take effect until restart.

### Sticky label state machine

A session's *display* label is not always the latest `label` you sent:

- `status = working`, `done`, or `idle`: the widget shows the **original prompt** (the label from the first `set working` that started the current task), falling back to the latest `label` if none was captured.
- `status = awaiting`: the widget shows the **current label** (e.g. the question being asked).
- `status = error`: the widget shows the current label (the error message).

A task boundary — transitioning from `done` or `idle` into `working` — resets the original prompt to whatever label the boundary `set` carried. An approval cycle — `working → awaiting → working` — preserves it. This lets you send short status noises like `"yes"` or `""` during an approval cycle without clobbering the user-visible task description.

If you POST `set` with **no `label` field**, the row keeps its previous label. Handy when toggling `status` on an already-running session.

### Examples

Curl:

```bash
curl -X POST http://127.0.0.1:9077/api/status \
  -H 'Content-Type: application/json' \
  -d '{"action":"set","id":"backup","status":"working","label":"nightly backup"}'

# ... do work ...

curl -X POST http://127.0.0.1:9077/api/status \
  -H 'Content-Type: application/json' \
  -d '{"action":"set","id":"backup","status":"done","label":"12 GB archived"}'
```

Any language with an HTTP client works the same way — build the JSON, POST it, move on.

### Features

- **Idempotent `set`**: same `id` updates in place; different `id`s coexist as separate rows.
- **Origin-guarded**: non-null `Origin` headers get `403`, blocking rogue web pages from flooding your widget.
- **Partial updates**: omit fields you don't want to change (except `id` and `status`, which are required).
- **Port configurable**: change `server_port` in `config.json` and restart.

### Standard features

- Always-on-top window, draggable by the header strip.
- System tray with show/hide toggle, always-on-top toggle, autostart toggle, save-position-on-exit toggle, open-config-file and open-log-file shortcuts.
- Color-coded state pills with a pulse animation on WAIT and ERROR.
- Sticky original-prompt label across approval cycles; same trigger resets the WORK accumulator on a new task.
- Config hot-reload from `config.json` on the next save — except `server_port`, which requires a restart.
