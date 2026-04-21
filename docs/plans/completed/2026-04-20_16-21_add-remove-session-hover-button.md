# Add "remove session" hover button

## Context

The dashboard has no way to dismiss an agent row from the UI. Stale or dead sessions (agent crashed, transcript path gone, hook stopped firing) linger forever because the only removal path today is the HTTP `Clear` action — which external tools rarely send. The user wants a manual cleanup affordance, but removal is rare enough that it shouldn't clutter the always-on-top widget.

Chosen UX (confirmed with user):
- **Row hover reveals a red X that overlays the timer.** Hover anywhere on the row → timer fades out, X fades in at the same position. Click X → row is removed immediately, no confirmation.
- If the underlying agent is still alive and its hook POSTs again, the row simply reappears on the next event. This is acceptable — it's effectively "hide until next heartbeat", which is the right behavior for stuck/stale rows.

## Backend — Rust (`src-tauri/`)

The state layer already supports removal: `AppState::apply_clear(&id)` at `src-tauri/src/state.rs:114-117` retains all sessions whose id doesn't match, and `WatcherRegistry::stop(&id)` cleans up the transcript watcher. The HTTP `Clear` handler at `src-tauri/src/http_server.rs:133-137` is the reference implementation — we mirror it in a Tauri command.

**New command** in `src-tauri/src/commands.rs` (append after `quit_app`, before `now_ms`):

```rust
#[tauri::command]
pub fn remove_session(id: String, app: AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.apply_clear(&id);
    }
    if let Some(reg) = app.try_state::<crate::log_watcher::WatcherRegistry>() {
        reg.stop(&id);
    }
    emit_sessions_updated(&app);
}
```

Unit `Result` return (not `Result<(), String>`) because both operations are infallible no-ops when state/registry are missing, matching the fire-and-forget style of `quit_app`.

**Register command** in `src-tauri/src/lib.rs:24-31` — add `commands::remove_session` to the `generate_handler!` list.

**Test** — extend the existing `#[cfg(test)] mod tests` in `state.rs` only if needed; `clear_removes_session` at `state.rs:253-261` already covers the state mutation. The new command is a thin wrapper — no new test required.

No capability changes needed. Custom Tauri commands don't require an ACL entry (verified against `src-tauri/capabilities/default.json`).

## Frontend — Svelte 5 (`src/`)

**API wrapper** in `src/lib/api.ts` (append after `quitApp`):

```ts
export function removeSession(id: string): Promise<void> {
  return invoke('remove_session', { id })
}
```

Parameter name must be `id` to match the Rust command signature (Tauri auto-converts Rust `snake_case` → JS `camelCase`, but `id` is the same in both).

**Row component** `src/lib/components/SessionItem.svelte`:

1. Import `removeSession` from `../api` in the existing `<script>` block.
2. Add a click handler: `function onRemove(e: MouseEvent) { e.stopPropagation(); removeSession(session.id).catch(err => console.error('remove failed', err)) }`.
3. In the markup at line 34, wrap `<span class="time">` in a relative-positioned container with an overlapping `<button class="remove" onclick={onRemove} aria-label="Remove session">×</button>`.
4. Style:
   - Container `.time-slot`: `position: relative; min-width: 36px;` (inherits the existing timer min-width so layout doesn't shift).
   - `.remove`: `position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; opacity: 0; pointer-events: none; background: transparent; border: 0; color: #b91c1c; font-size: 14px; font-weight: 700; cursor: pointer; transition: opacity 120ms ease;`.
   - Reveal on row hover: `.row:hover .remove { opacity: 1; pointer-events: auto; }` and `.row:hover .time { opacity: 0; }` with matching transition on `.time`.
   - `.remove:hover { color: #ef4444; }` for press feedback.

   Use the × character (U+00D7), not a letter X — it's visually centered and matches the dismissiveness of an X button without needing an icon font.

Row-level `:hover` selectors already work because `.row` is the outermost element (line 29) and `SessionItem` is the child of the `SessionList` virtual list.

## Files modified

- `src-tauri/src/commands.rs` — add `remove_session` command (~10 lines).
- `src-tauri/src/lib.rs` — register command in `generate_handler!`.
- `src/lib/api.ts` — add `removeSession` wrapper.
- `src/lib/components/SessionItem.svelte` — hover-reveal button, styles, click handler.

No changes to `state.rs`, `http_server.rs`, `log_watcher.rs`, or capabilities.

## Verification

1. `pnpm tauri dev` (or the project's equivalent) — widget launches.
2. Seed a session via the existing HTTP API (same channel the hook uses):
   ```powershell
   $body = '{"action":"set","id":"test-row","status":"working","label":"dummy"}'
   Invoke-RestMethod -Uri http://127.0.0.1:9077/api/status -Method Post -Body $body -ContentType 'application/json'
   ```
3. Hover the row → timer fades, red × appears in its place. No layout shift.
4. Click × → row disappears immediately.
5. Move the pointer off the row between sessions → other rows show their timers normally.
6. Re-send the same POST → row reappears (confirms manual removal doesn't corrupt state).
7. Send a `set` with a `transcript_path`, then remove the row → confirm no "orphan watcher" messages in `widget.log` (`%APPDATA%\com.anothersava.ai-agent-dashboard\widget.log`). Reason: validates the `WatcherRegistry::stop` call in the new command.
8. `cargo test -p ai-agent-dashboard` (or whatever the src-tauri crate is named) — existing tests still green.
