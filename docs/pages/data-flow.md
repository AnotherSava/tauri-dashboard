---
layout: default
title: Data flow
---

[Home](..) | [Claude Code](claude-code) | [HTTP API](http-api) | [Development](development)

---

End-to-end: what happens when a Claude Code hook fires, when a transcript file gets a new line, or when you toggle a tray menu item.

## The three input sources

```
┌──────────────────┐    POST /api/status    ┌────────────────┐
│  External agent  │──────────────────────▶ │  axum (Rust)   │
│  (hook, curl,    │                        │  :9077         │
│   script)        │                        └───────┬────────┘
└──────────────────┘                                │ apply_set
                                                    │ / apply_clear
                                                    ▼
┌──────────────────┐   notify::Event       ┌────────────────┐
│  transcript      │─────────────────────▶ │  AppState      │     app.emit
│  <session>.jsonl │                       │  Mutex<Vec<    │───────────────▶ Svelte
└──────────────────┘                       │    AgentSession│   "sessions_      (listen)
                                           │  >>            │    updated")
                                           └────────┬───────┘
┌──────────────────┐  #[tauri::command]             │
│  Svelte UI       │────────────────────▶ commands.rs ──apply_clear──▶ AppState
└──────────────────┘                                │
                                                    ▼
                                            Window / TrayIcon
                                            native APIs

┌──────────────────┐  file change event
│  config.json     │─────────────────▶ config_watcher reloads ─▶ emit("config_updated")
└──────────────────┘                                                       │
                                                                           ▼
                                                                  Svelte + tray refresh
```

Every mutation to session state funnels through `state::apply_set` or `state::apply_clear` so the sticky-label rules, working-time accumulator, and upgrade-only merge policy are enforced in one place regardless of origin.

## Path 1 — Hook POSTs status

1. Claude Code fires a lifecycle event (`UserPromptSubmit`, `Stop`, etc.). The hook command spawns `python claude_hook.py <arg>` and pipes the event payload to stdin.
2. `claude_hook.py` reads `config.json` from the widget's app data dir for `projects_root`, `benign_closers`, `server_port`. Derives the session `id` from `cwd` relative to `projects_root`. Calls `classify(arg, payload, benign_closers)` to map argv + payload to a `(status, label)` pair.
3. `build_body(...)` produces the POST body: `{action, id, status, source, label?, transcript_path?, updated}`.
4. `POST /api/status` hits the axum handler. Origin guard rejects non-null cross-origin requests. Body deserializes to `StatusRequest::Set`.
5. `AppState::apply_set` runs. If status transitions out of `working`, it accumulates elapsed time into `working_accumulated_ms`. If the transition is a task boundary (`done` / `idle` → `working`), it re-captures `original_prompt` and zeroes the accumulator. Otherwise the existing `original_prompt` is preserved.
6. If the payload carries `transcript_path`, `WatcherRegistry::start` spawns a per-session tokio task with a `notify::RecommendedWatcher` on the transcript's parent directory.
7. `emit_sessions_updated` broadcasts the fresh snapshot on the `sessions_updated` event.
8. The Svelte frontend's `listen` callback replaces its `$state` sessions array, Svelte's reactivity re-renders the list, the row updates within a frame.

## Path 2 — Transcript-driven updates

1. The watcher task from Path 1 is listening to filesystem events on the transcript's parent directory.
2. Claude Code writes a new JSONL line to the transcript. `notify` fires a `Modify` event; the watcher filters to events matching the exact transcript path.
3. The task sends a drain signal over an mpsc channel to itself. A 150ms debouncer collapses bursts (editors / streaming writes often produce several events per logical change).
4. `drain` reads the new bytes from the tracked byte offset, joins with leftover content from the previous drain, and splits into complete JSONL lines + a new leftover for the next call.
5. `infer_state` walks the new lines newest-first, skipping non-conversational entries (metadata, sidechains, synthetic errors). Returns the current `state`, latest `model`, and latest summed input-side token count.
6. `apply_watcher_update` merges the inference into the session: watcher can set status to `working`, update `model`, update `input_tokens`, but cannot roll a session back to `done`, `idle`, `awaiting`, or `error` — hook events stay authoritative for terminal states. This avoids the race where the watcher reads a trailing assistant text as "done" while a fresh turn is already in flight.
7. If anything changed, the session's `updated` timestamp refreshes and `emit_sessions_updated` fires exactly as in Path 1.

The initial drain on watcher startup suppresses the inferred **state** (a resume would otherwise snap to a stale "done" from the prior turn) but still surfaces model and token counts.

Tauri commands have two possible targets: native window/tray APIs (`hide_window`, `show_window`, `toggle_window`, `quit_app`) or `AppState` itself — `remove_session` calls `apply_clear` to dismiss a row the user no longer cares about, then re-emits the snapshot on the same `sessions_updated` channel.

## Path 3 — Tray toggles

1. User clicks "Always on top" in the tray menu. `muda` fires a `MenuEvent` with the item's id.
2. The tray handler calls `window.set_always_on_top(new_state)` directly on the native window — no IPC round-trip.
3. `ConfigState::with_mut` flips `always_on_top` in the managed config. `ConfigState::save_to_disk` writes `config.json`.
4. The tray's `CheckMenuItem::set_checked` syncs the visual checkmark.
5. `emit_config_updated` broadcasts the new config. The frontend picks up the updated color thresholds, token-window lookup, and (future-proof) any UI-driving fields.

## Path 4 — External config edits

1. User edits `config.json` directly (via the "Open config/logs location" tray shortcut or any editor).
2. `config_watcher` — a `notify::RecommendedWatcher` on the config directory — receives a `Modify` event.
3. The 150ms debouncer waits for any rename-based atomic writes to settle.
4. `Config::load_or_default` re-reads the file. Serde serializes both the new and current in-memory configs to JSON strings; if they're byte-identical, the reload is skipped — this is how our own tray writes avoid re-triggering the reload path.
5. `apply_config_to_window` applies runtime-safe changes (always-on-top, saved window position). Port changes are intentionally ignored on hot-reload and require a restart.
6. `config_updated` is emitted and the tray check marks re-sync.

## Sticky-label state machine

| Existing session? | Prior status             | New status | Action on `original_prompt` |
|---                |---                       |---         |---                          |
| no                | —                        | `working`  | set to incoming `label`     |
| no                | —                        | anything   | leave `None`                |
| yes               | `None` / `done` / `idle` | `working`  | **re-capture** to `label` (new task); reset `working_accumulated_ms = 0` |
| yes               | any                      | any        | leave pinned                |

UI display rule:

| Status                      | Label shown                                         |
|---                          |---                                                  |
| `awaiting`                  | current `label` (the agent's question)              |
| `error`                     | current `label` (the error message from the agent)  |
| `working` / `done` / `idle` | `original_prompt` if set, else current `label`      |

This is why an approval cycle — `working → awaiting → working` with `label = "yes"` — keeps "fix foo.py" visible across the round-trip, while a genuinely new task after `done` gets a fresh display.
