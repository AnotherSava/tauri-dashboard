# tauri-dashboard

Tauri v2 desktop dashboard application.

## Project

An always-on-top Windows widget that tracks live status of AI coding agents. A Rust backend (`src-tauri/`) hosts the Tauri v2 app, an embedded `axum` HTTP server on `127.0.0.1:9077` that accepts status POSTs from external agents, a `notify`-backed transcript watcher that tails Claude Code JSONL for live token updates, and a system tray. A Svelte 5 + Vite frontend (`src/`) renders the session list and subscribes to Tauri events for live updates. A Python hook (`integrations/claude_hook.py`) bridges Claude Code's lifecycle events to the HTTP API, and `tests/test_claude_hook.py` covers its classify / chat-id / config-path logic. Runtime state lives at `%APPDATA%\com.anothersava.ai-agent-dashboard\` (`config.json` for settings, `widget.log` for tracing output).

Key module map in `src-tauri/src/`: `state.rs` (sticky-label state machine), `config.rs` (load/save), `config_watcher.rs` (notify-backed hot-reload), `http_server.rs` (axum handler), `log_watcher.rs` (transcript tailing + merge policy), `tray.rs` (menu + autostart), `commands.rs` (Tauri commands + event emitters), `logging.rs` (tracing setup), `notifications.rs` (1s-tick reconciler + `Notifier` trait), `telegram.rs` (reqwest-based Telegram Bot API client), `usage_limits.rs` (Anthropic OAuth usage poller for 5h / 7d rate-limit bars). All Rust tests live in `#[cfg(test)]` modules next to the code they cover.

## Skills

The `.claude/skills/tauri-*` skills were vendored from https://github.com/dchuk/claude-code-tauri-skills — they are not a verbatim copy. Five skills (`tauri-architecture`, `tauri-capabilities`, `tauri-plugins`, `tauri-project-setup`, `tauri-testing`) were updated with selective improvements from https://github.com/dchuk/claude-code-tauri-skills/pull/2: action-oriented descriptions with explicit trigger keywords, numbered workflows, and build/verify checkpoints. A few reference examples the PR removed were preserved (platform webview table in `tauri-architecture`, extra window/platform examples in `tauri-capabilities`).

Additional hands-on notes from building this project were layered on top of the vendored content:
- `tauri-project-setup` — warns that `create-tauri-app`'s `svelte-ts` template is SvelteKit, not plain Svelte
- `tauri-capabilities` — documents `core:window:allow-start-dragging` for frameless drag regions
- `tauri-system-tray` — adds a `CheckMenuItem` managed-handle pattern for syncing tray checkboxes with their backing state
- `tauri-app-resources` — explains migrating from an existing `.ico` via Pillow and stripping the `android/`/`ios/` output for desktop-only projects
- `tauri-windows-distribution` — documents GitHub Releases' space-to-dot filename normalization for release assets

Upstream has no LICENSE file yet — tracked in https://github.com/dchuk/claude-code-tauri-skills/issues/3. Attribution is preserved here; the vendored content will be relicensed as soon as upstream publishes terms.

The connection to the upstream repo is not maintained — no submodule, no auto-sync. To refresh, re-clone upstream and re-merge manually.
