# Fix startup window flash (top-left white → saved position dark)

## Context

On launch, the widget briefly appears at screen top-left with a white background, then jumps to its saved/default position and flips to the dark UI. Two independent causes, confirmed by reading the code:

1. **Position flash.** `tauri.conf.json` has no `"visible": false`, so Tauri shows the window immediately at its default (0,0). The setup hook in `src-tauri/src/lib.rs:55-65` applies the saved / default-bottom-right position *after* the window is already on screen, producing the jump.
2. **White flash.** `tauri.conf.json` has no `backgroundColor`, so the native window paints system white until the webview renders. The Svelte CSS is correct — `:global(html, body) { background: transparent }` and `.widget { background: #1c1c1e }` in `src/App.svelte:45-62` — but nothing backs those during the few hundred ms before first paint.

Goal: window stays hidden until position is set and the Svelte app has painted at least once; if anything goes wrong, a safety-net timeout still reveals it.

## Approach

Standard Tauri "show-on-ready" pattern. The existing `show_window` command (`src-tauri/src/commands.rs:20-24`) already does `show() + set_focus()` — reuse it rather than adding a new command.

### 1. `src-tauri/tauri.conf.json` (window block at lines 13-24)

- Add `"visible": false` — window stays hidden at creation.
- Add `"backgroundColor": "#1c1c1e"` — matches `.widget` dark, so even if something shows the window early there's no white flash.

### 2. `src-tauri/src/lib.rs` setup hook (after line 64, inside the `if let Some(window)` block)

Add a safety-net: spawn an async task that waits ~1500 ms and shows the window if it's still hidden. Prevents a broken frontend from leaving the widget permanently invisible. Sketch:

```rust
let window_clone = window.clone();
tauri::async_runtime::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    if matches!(window_clone.is_visible(), Ok(false)) {
        let _ = window_clone.show();
    }
});
```

(`tokio` is already a transitive dep via Tauri's async runtime; if this doesn't resolve, use `tauri::async_runtime::spawn` + an equivalent timer. Verify during implementation.)

### 3. `src/App.svelte` (`onMount`, lines 11-32)

After `config` and `sessions` are loaded, `await tick()` to let Svelte flush the first render, then call `invoke('show_window')`. Import `invoke` from `@tauri-apps/api/core` and `tick` from `svelte`. Place the call inside the existing IIFE after the initial fetches succeed so the window is only revealed once the UI is real.

Intentionally kept outside `src/lib/api.ts` — this is a one-off bootstrap call, not part of the app's data API surface.

## Critical files

- `src-tauri/tauri.conf.json` — window config (lines 13-24)
- `src-tauri/src/lib.rs` — setup hook (lines 32-78)
- `src/App.svelte` — onMount (lines 11-32)

Unchanged but relevant:
- `src-tauri/src/commands.rs:20-24` — existing `show_window` command (reused)
- `src-tauri/src/config_watcher.rs:103-115` — `apply_default_position` still runs pre-show
- `src-tauri/src/tray.rs` — tray "Show Window" toggle keeps working; `window.show()` on a hidden window is idempotent

## Verification

1. `deploy` (build + install + launch).
2. Watch the first launch: window should appear directly at its target position with dark background — no top-left flash, no white frame.
3. Toggle `save_window_position` in `%APPDATA%\com.anothersava.ai-agent-dashboard\config.json` between `true` (with a saved `window_position`) and `false` (default bottom-right) and relaunch — both paths should show no flash.
4. Use the tray "Show/Hide" toggle after startup — should still hide and show correctly.
5. Regression check for the safety net: temporarily comment out the `invoke('show_window')` call, relaunch, confirm the window appears after ~1.5 s via the timeout, then restore the call.
