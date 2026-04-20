# Migrate `ai-agent-dashboard` from Electron to Tauri v2

## Context

`ai-agent-dashboard` (at `../ai-agent-dashboard`) is a single-window, always-on-top Windows widget (~1500 LOC, vanilla JS on Electron v41) that shows live status of AI coding agents. It accepts status POSTs on `127.0.0.1:9077`, tails Claude Code transcript JSONL files, and renders a color-coded session list with a context-usage bar and a system tray.

We're rewriting it in Tauri v2 in this repo (`tauri-dashboard`). Goals:

1. **Smaller, faster binary** — Tauri produces ~5–10 MB native executables (vs. Electron's 100+ MB), which matters for an always-running widget.
2. **Simpler feature set** — drop peripheral features, collapse SSE into Tauri's built-in event channel, remove the dual config/build layout.
3. **Learn Tauri + Rust on a small, well-understood app** — your background is Java / Python / some JS; Rust will be new.

Migration is **iterative**. Each stage ends with a runnable app and a commit. The Electron app at `../ai-agent-dashboard` stays untouched until the Tauri v1 is feature-complete, then it's retired.

## Decisions

| Decision | Choice |
|---|---|
| Frontend framework | **Svelte + Vite** |
| Config location | **App data dir** (`%APPDATA%/ai-agent-dashboard/config.json`) |
| Distribution | **NSIS installer only** |
| Config UX | **Tray menu for toggles; `config.json` for everything else; hot-reload on save; no settings panel** |
| Default window position | **Bottom-right of primary monitor**, small margin |
| Position handling | Default on first launch; `save_window_position` flag persists last position on exit |
| HTTP server port | **Configurable in `config.json`** (`server_port`, default 9077); port change requires restart |
| Python `claude_hook.py` | **Keep unchanged for v1** |

### Dropped in v1

Codex shell wrapper • OpenWebUI filter • MCP server • desktop notifications • auto-dismiss timer • per-source icons (Claude-only for v1) • `thinking` status • 4-corner position selector • `openUrl` config action • in-widget settings panel.

### Kept in v1

Sessions list (Claude Code only, structure is source-extensible) • color-coded status badges • **sticky original-prompt label** (see state machine below) • context-usage bar with config-driven thresholds • system tray with toggle + autostart + utility items • `POST /api/status` HTTP API with the same payload as today • Claude Code transcript watcher • Python `claude_hook.py` unchanged • `config.json` with hot-reload.

#### Sticky label state machine

Each `AgentSession` carries both `label` (latest value from a `set` POST) and `original_prompt: Option<String>` (the pinned task description).

Transitions (on `POST /api/status action=set` with `status=S`, `label=L` for session `id=I`):

| Existing session? | Prior status | New status | Action on `original_prompt` |
|---|---|---|---|
| no | — | `working` | set to `L` |
| no | — | anything else | leave `None` |
| yes | `None` / `done` / `idle` | `working` | **re-capture** to `L` (new task starts) |
| yes | any | any | leave pinned |

Display rule in UI:

| Status | Label shown |
|---|---|
| `awaiting` | current `label` (the agent's question) |
| `error` | current `label` (the error message — usually from the agent) |
| `working` / `done` / `idle` | `original_prompt` if set, else `label` |

Rationale: during an approval cycle the dashboard reads "fix foo.py", not "yes" / "go" / "Can I run bash X?" — you can glance at the widget and see what you asked for, not what the agent is momentarily blocked on. On `error`, the error message is what matters (and is what the hook puts in `label`). `done` is the task boundary; the next `working` after `done` starts a new sticky capture.

### Designed for, not implemented in v1 (hooks preserved, feature dormant)

- **Multi-agent sources.** Keep `source` on the POST payload and the `AgentSession` struct but render Claude-only styling. Adding a second agent type later = new CSS + new value for `source`, no schema change.
- **`benign_closers`.** Lives in `config.json`, read by `claude_hook.py`, not by the widget. Keep the field in the config file so the hook keeps working; the widget itself ignores it.

### Future iterations (out of v1 scope)

- Linux / macOS builds.
- Auto-updater.
- Code signing.

## Concepts you'll meet in Tauri (framed for a Java/Python dev)

1. **Two processes, one app.** The Rust binary is the "host" (think: a JVM process with direct OS access). It opens a native WebView (Microsoft Edge's WebView2 on Windows) and loads your Svelte UI inside. The UI is HTML/CSS/JS like a web app, but the window is native — no browser chrome, no localhost server.

2. **Commands = RPC methods.** Svelte calls `invoke("get_sessions")`, Rust exposes `#[tauri::command] fn get_sessions(...) -> Vec<AgentSession>`. Serialization is automatic via `serde` (Rust's equivalent of Python's `pydantic` / Jackson in Java).

3. **Events = message bus.** Rust pushes updates: `app_handle.emit("sessions_updated", payload)`. Svelte listens: `listen("sessions_updated", evt => ...)`. This replaces the Electron app's HTTP/SSE stream — it's lighter, built-in, and typed.

4. **Capabilities = fine-grained permissions.** You list in a JSON file exactly what the frontend is allowed to ask for (e.g., "read files under `$APPDATA/ai-agent-dashboard`", "control the main window"). Any call outside that list is rejected by the runtime. Think of it as AWS IAM for the webview-to-Rust bridge.

5. **Async Rust.** Tauri runs on `tokio` (Rust's async runtime, roughly like Python's `asyncio`). The HTTP server, file watcher, and state reads all live on tokio tasks. Shared state uses `Arc<Mutex<T>>` — `Arc` = thread-safe refcount (like Java's `AtomicReference` to an immutable holder), `Mutex` = the usual lock.

6. **Binary size / startup.** Release builds use LTO + stripped symbols. A widget like ours should land in single-digit MB and launch in <500 ms cold.

7. **WebView2 on Windows.** Ships with Edge (so it's on every modern Windows). NSIS installer is configured with `webviewInstallMode: "downloadBootstrapper"` — if the user doesn't have it, the installer grabs it.

## Target Architecture

```
tauri-dashboard/
├── src/                              Svelte frontend (Vite)
│   ├── App.svelte                    main widget (list + drag region)
│   ├── lib/
│   │   ├── api.ts                    invoke() + listen() wrappers
│   │   ├── types.ts                  AgentSession / Config TypeScript types
│   │   └── components/
│   │       ├── SessionList.svelte    renders the list of active sessions
│   │       ├── SessionItem.svelte    one row: label, status, context bar
│   │       ├── ContextBar.svelte     gradient fill by % of context used
│   │       └── StatusBadge.svelte    color dot + status text
│   ├── main.ts
│   └── styles.css
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   ├── icons/                        generated from existing .ico
│   └── src/
│       ├── main.rs                   entry (tiny: calls lib::run())
│       ├── lib.rs                    Builder, setup(), plugin wiring
│       ├── state.rs                  AppState, AgentSession, merge policy
│       ├── config.rs                 load/save config + hot-reload watcher
│       ├── commands.rs               #[tauri::command] functions
│       ├── http_server.rs            axum on 127.0.0.1:<port>
│       ├── log_watcher.rs            transcript JSONL watcher + parser
│       └── tray.rs                   TrayIconBuilder + menu handlers
├── package.json
└── vite.config.ts
```

### Data flow

```
┌───────────────────┐  POST /api/status   ┌───────────────┐
│ claude_hook.py    │───────────────────▶ │ axum server   │
│ (unchanged)       │                     │               │
└───────────────────┘                     └──────┬────────┘
                                                 │ mutate
┌───────────────────┐  notify events             ▼
│ transcript *.jsonl│───────────────────▶ ┌───────────────┐  emit("sessions_updated")
│                   │                     │   AppState    │─────────────────▶ Svelte
└───────────────────┘                     │ Arc<Mutex<…>> │                  (listen)
                                          └──────┬────────┘
                                                 │
┌───────────────────┐  invoke(cmd, …)             │
│ Svelte UI         │────────────────────▶ commands.rs
└───────────────────┘                             │
                                                  ▼
                                          window / TrayIcon APIs

┌───────────────────┐  file change event
│ config.json       │───────────────▶ config.rs reloads → emit("config_updated")
└───────────────────┘                                          │
                                                               ▼
                                                        Svelte + tray refresh
```

### Rust crates

| Crate | Role | Analogy |
|---|---|---|
| `tauri` (v2) | desktop framework | Spring Boot, but for native windows |
| `tauri-plugin-fs` | scoped file access for frontend | sandboxed file IO |
| `tauri-plugin-single-instance` | one widget at a time | `lockfile` on a pidfile |
| `tauri-plugin-autostart` | "Open on system start" toggle | Windows Startup folder / registry wrapper |
| `axum` + `tokio` | HTTP server | Flask / aiohttp |
| `serde` + `serde_json` | (de)serialization | pydantic / Jackson |
| `notify` | file-system watcher | watchdog (Python) |
| `tracing` + `tracing-appender` | JSONL logs | Python's `logging` with JSON formatter |

We explicitly **omit** `tauri-plugin-shell` and `tauri-plugin-notification` (no `openUrl`, no notifications in v1).

### Shared state merge policy

Ported conceptually from `chat-state.cjs` / `log-watcher.cjs`, trimmed of dead rules:

- `POST /api/status set/clear` is **authoritative** for all status transitions.
- Transcript watcher may **upgrade** a session to `working` only (never set `done`, `idle`, `awaiting`, `error`).
- Sticky-label state machine (see "Kept in v1" above) runs on every `set` POST before events are emitted.

### Config file schema (v1)

```json
{
  "server_port": 9077,
  "always_on_top": true,
  "save_window_position": false,
  "window_position": null,
  "context_window_tokens": {
    "claude-opus-4-7": 200000,
    "claude-sonnet-4-6": 200000,
    "claude-haiku-4-5": 200000
  },
  "context_bar_thresholds": [
    { "percent": 0,  "color": "#3a7c4a" },
    { "percent": 60, "color": "#c6a03c" },
    { "percent": 85, "color": "#c64a4a" }
  ],
  "benign_closers": ["What's next?", "Anything else?"]
}
```

Notes:
- `server_port` change requires app restart (we don't juggle socket lifecycle for a rarely-changed field).
- `window_position` is written only when `save_window_position: true` and only on clean exit.
- `benign_closers` is read by `claude_hook.py`, not the widget; kept here so there's one config file.
- Hot-reload: on `config.json` change, the widget reloads, validates, emits `config_updated`, and re-applies everything except `server_port`.

### Tray menu (v1)

```
Show / Hide                ← click icon or this item toggles window
──────────────────
✓ Always on top            ← writes always_on_top to config.json
✓ Save position on exit    ← writes save_window_position to config.json
✓ Open on system start     ← toggles autostart via tauri-plugin-autostart
──────────────────
Open config file           ← opens config.json in default editor
Open log file              ← opens widget.log in default viewer
──────────────────
About                      ← small dialog with name/version
Quit
```

"Show / Hide" is also bound to left-click on the tray icon (Windows standard behavior). Autostart checkbox reads/writes the Windows Startup registry entry via `tauri-plugin-autostart`; no config.json field needed (OS is the source of truth).

## Iterative Stages

Each stage ends with a runnable build and a commit. Stages 0–5 should take a day or less each; Stage 6 is the longest (Rust + JSONL parsing + file-watch correctness).

### Stage 0 — Scaffold
- `pnpm create tauri-app` → Svelte + TypeScript.
- Configure `tauri.conf.json`: `decorations: false`, `alwaysOnTop: true`, `width: 420`, `height: 320`, `skipTaskbar: false`, `resizable: true`.
- Capabilities: core defaults only.
- Commit.
- **Verify:** `pnpm tauri dev` opens a blank frameless window that stays on top.
- Skills: `tauri-project-setup`, `tauri-frontend-js`, `tauri-configuration`.

### Stage 1 — UI skeleton (free to simplify / improve visuals)
- Build Svelte components: `SessionList`, `SessionItem`, `ContextBar`, `StatusBadge`.
- Mock 3–4 sessions covering all status values and context-bar thresholds.
- Mock one session in `awaiting` state **and** one transitioning back to `working` to verify the sticky-label rule visually.
- Drag region on the titlebar via `data-tauri-drag-region`.
- This is the place to simplify the visuals — no parity requirement.
- **Verify:** mocked UI looks right; dragging works; sticky label shows the right text for each status.
- Skill: `tauri-window-customization`.

### Stage 2 — Rust state + Tauri commands/events
- `state.rs`: `AgentSession` struct (fields: `id`, `status`, `label`, `original_prompt`, `source`, `model`, `input_tokens`, `updated`) + `AppState { sessions: Mutex<Vec<AgentSession>> }`.
- Implement sticky-label state machine inside `state.rs::apply_set(...)` so every entry point funnels through one place.
- Unit tests for the state machine in `#[cfg(test)]`: new session, mid-task awaiting, approval-cycle recovery, done→new-task reset.
- Commands: `get_sessions`, `hide_window`, `show_window`, `toggle_window`, `quit_app`.
- Emit `sessions_updated` whenever state changes.
- Svelte switches from mock data to `invoke("get_sessions")` + `listen("sessions_updated")`.
- Seed 1–2 hardcoded sessions at startup to verify end-to-end.
- **Verify:** invoking hide/show via a debug button (temporary) works; sticky-label unit tests pass.
- Skills: `tauri-calling-rust`, `tauri-calling-frontend`, `tauri-frontend-events`.

### Stage 3 — HTTP server
- `http_server.rs`: axum on `127.0.0.1:<server_port>`, spawned from `setup()`.
- `POST /api/status` with same payload shape as today (`action: "set" | "clear" | "config"`).
- Origin guard: reject non-null `Origin` header (same as Electron version).
- Shared `AppHandle` for emitting events after state mutations.
- **Verify:** run existing `../ai-agent-dashboard/integrations/claude_hook.py` against the Tauri dev build; sessions appear in the UI and the sticky-label rule holds through a real approval cycle.

### Stage 4 — System tray
- `tray.rs`: `TrayIconBuilder` with the menu above.
- Left-click → toggle window. Menu items wired to commands.
- "Open on system start" toggles autostart via `tauri-plugin-autostart` (Windows Startup folder).
- "Open config file" / "Open log file" use `std::process::Command` with `cmd /c start ""` on Windows (no shell plugin needed).
- **Verify:** all menu items behave; left-click toggles visibility; autostart survives reboot.
- Skill: `tauri-system-tray`.

### Stage 5 — Config load + save + hot-reload
- `config.rs`: `Config` struct with `serde`, `load()`/`save()` against `app_data_dir()/config.json`.
- Create defaults on first launch.
- `notify` watcher on `config.json` → reload → emit `config_updated`.
- Tray toggles and position-save write back to `config.json`.
- HTTP `action: "config"` continues to work (updates in-memory + writes to file).
- Default window position (bottom-right) applied on launch unless `save_window_position && window_position` is set.
- **Verify:** edit config.json externally → widget picks up changes without restart (except `server_port`).
- Skills: `tauri-app-resources`, `tauri-scope`.

### Stage 6 — Transcript watcher (Rust port, longest stage)
- `log_watcher.rs`: per-session `notify::RecommendedWatcher`, tracks byte offset per file, incremental JSONL parse.
- Port state inference from `log-watcher.cjs`: user msg → `working`, assistant `tool_use` → `working`, assistant text-only → don't touch (hook is authoritative for `done`). Drop `thinking`.
- Extract model + input token count from assistant `usage` block.
- Apply merge policy (upgrade-only).
- Unit tests in `#[cfg(test)]` mirroring `tests/log-watcher.test.cjs` + `tests/chat-state.test.cjs`.
- **Verify:** manually append to a transcript file → UI updates; hook-sent `done` is not overridden by the watcher.
- Skill: `tauri-testing`.

### Stage 7 — NSIS installer + icons
- Generate Windows icons from `../ai-agent-dashboard/assets/ai-agent-dashboard.ico`.
- `tauri.conf.json`: `bundle.targets: ["nsis"]`, `productName`, `appId: "com.anothersava.ai-agent-dashboard"`, `version`.
- `webviewInstallMode: "downloadBootstrapper"`.
- `pnpm tauri build` → installer in `src-tauri/target/release/bundle/nsis/`.
- **Verify:** install on a clean Windows VM, launch, run `claude_hook.py` against it, reboot, confirm widget relaunches cleanly.
- Skills: `tauri-windows-distribution`, `tauri-binary-size`.

### Stage 8 — Polish (optional)
- `tracing` JSONL logs to `app_data_dir()/widget.log`.
- Minimal README + migration notes (config path changed from `config/config.json` to `%APPDATA%/ai-agent-dashboard/config.json`).
- GitHub Actions release pipeline if desired (`tauri-pipeline-github`).

## Reference files in the old project (read-only during this migration)

| File | Why we look at it |
|---|---|
| `../ai-agent-dashboard/src/widget.html` | Layout + CSS reference (not mandatory to match) |
| `../ai-agent-dashboard/src/widget.cjs` | HTTP server logic, tray wiring, window handling |
| `../ai-agent-dashboard/src/log-watcher.cjs` | JSONL parser + state inference to port to Rust |
| `../ai-agent-dashboard/src/chat-state.cjs` | Merge policy to port to Rust |
| `../ai-agent-dashboard/config/config.example.json` | Config schema baseline |
| `../ai-agent-dashboard/tests/*.test.cjs` | Test cases to re-express as Rust unit tests |
| `../ai-agent-dashboard/integrations/claude_hook.py` | The Python hook we keep running unchanged |

## End-to-end verification (at Stage 6)

1. Launch the Tauri dev build.
2. Point Claude Code at the existing `integrations/claude_hook.py` targeting `http://127.0.0.1:9077/api/status`.
3. Run a Claude task and confirm: `idle → working → done` transitions, context bar fills, tray menu items work.
4. Manually append an assistant message to a transcript file; confirm the watcher upgrades state but does not override hook-sent terminal states.
5. Edit `%APPDATA%/ai-agent-dashboard/config.json` (change a threshold color); confirm UI hot-reloads.
6. Exit with `save_window_position: true`; relaunch; confirm position restored.

After Stage 7: repeat step 2 against the installed build on a clean machine.
