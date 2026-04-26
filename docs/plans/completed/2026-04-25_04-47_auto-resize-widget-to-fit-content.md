# Auto-resize widget to fit content

## Context

The dashboard is a fixed 420×320 always-on-top widget. As session items are added or removed, content scrolls inside that fixed window — useful information is hidden below the fold or surrounded by dead space. The user wants the window to auto-fit content height while keeping a chosen edge anchored:

- **Up** — bottom edge stays put, top moves (window grows upward).
- **Down** — top edge stays put, bottom moves (window grows downward).
- **None** — current behavior (manual size, content scrolls).

The mode lives in the system tray right-click menu (the existing menu surface — there is no in-webview context menu today). Switching modes snaps the window to fit current content immediately. While Up/Down is active, the user can still drag horizontal edges; vertical drag is locked via `min_height == max_height`.

## Approach

### Config schema — `src-tauri/src/config.rs`

Add a `snake_case` enum and a Config field:

```rust
#[derive(Default, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoResize {
    #[default]
    None,
    Up,
    Down,
}
```

```rust
pub struct Config {
    // ...existing fields...
    #[serde(default)]
    pub auto_resize: AutoResize,
}
```

Update `Config::default()` (line 85) and the schema-default test (lines 171–217) so a config missing `auto_resize` deserializes to `None`.

### Tray submenu — `src-tauri/src/tray.rs`

Build a submenu with three `CheckMenuItem`s (one checked at a time) and pass it to the existing `Menu::with_items` call. `Submenu` implements `IsMenuItem`, so no migration to `MenuBuilder` is needed.

```rust
let auto_none = CheckMenuItem::with_id(app, MENU_AUTO_RESIZE_NONE, "None", true, mode == AutoResize::None, None::<&str>)?;
let auto_up   = CheckMenuItem::with_id(app, MENU_AUTO_RESIZE_UP,   "Up",   true, mode == AutoResize::Up,   None::<&str>)?;
let auto_down = CheckMenuItem::with_id(app, MENU_AUTO_RESIZE_DOWN, "Down", true, mode == AutoResize::Down, None::<&str>)?;
let auto_resize = SubmenuBuilder::new(app, "Auto resize").items(&[&auto_none, &auto_up, &auto_down]).build()?;
```

Extend `TrayHandles` (lines 22–26) with `auto_resize_none`, `auto_resize_up`, `auto_resize_down`. Click handler (`select_auto_resize_mode(app, mode)`) follows `toggle_always_on_top` (lines 130–144):

1. `state.with_mut(|c| c.auto_resize = mode)` + `save_to_disk()`.
2. `set_checked(true)` on the picked item, `set_checked(false)` on the other two.
3. `emit_config_updated(app)`.
4. If `mode != None`, do **not** snap from Rust — the frontend's `$effect` watching `config.auto_resize` triggers the snap with a freshly measured height (avoids the flash of pre-measured size).

### Resize math — new module `src-tauri/src/auto_resize.rs`

```rust
pub fn apply(window: &WebviewWindow, mode: AutoResize, desired_logical_height: f64) -> Result<()> {
    if matches!(mode, AutoResize::None) {
        clear_constraints(window)?;
        return Ok(());
    }
    let scale = window.scale_factor()?;
    let pos = window.outer_position()?;          // PhysicalPosition<i32>
    let size = window.outer_size()?;             // PhysicalSize<u32>
    let new_height_phys = (desired_logical_height * scale).round() as i32;
    let current_height_phys = size.height as i32;

    let new_y = match mode {
        AutoResize::Up   => pos.y + (current_height_phys - new_height_phys),
        AutoResize::Down => pos.y,
        AutoResize::None => unreachable!(),
    };

    // Clamp y to the current monitor's top edge (multi-monitor safe).
    let monitor_top = window.current_monitor()?.map(|m| m.position().y).unwrap_or(0);
    let new_y = new_y.max(monitor_top);

    // Lock height; allow horizontal resize.
    let monitor_width_logical = window.current_monitor()?
        .map(|m| m.size().width as f64 / scale).unwrap_or(3840.0);
    window.set_min_size(Some(LogicalSize::new(280.0, desired_logical_height)))?;
    window.set_max_size(Some(LogicalSize::new(monitor_width_logical, desired_logical_height)))?;

    let current_width_logical = size.width as f64 / scale;
    window.set_size(LogicalSize::new(current_width_logical, desired_logical_height))?;
    window.set_position(PhysicalPosition::new(pos.x, new_y))?;
    Ok(())
}

pub fn clear_constraints(window: &WebviewWindow) -> Result<()> {
    window.set_max_size::<LogicalSize<f64>>(None)?;
    window.set_min_size(Some(LogicalSize::new(280.0, 200.0)))?;
    Ok(())
}
```

Notes:
- Mix of physical (position) and logical (size) is intentional — each Tauri setter takes whatever you hand it. Round, don't truncate.
- `LogicalSize::new(...)` — no struct-literal form exists.

### New Tauri command — `src-tauri/src/commands.rs`

```rust
#[tauri::command]
pub fn apply_auto_resize(height: f64, app: AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return; };
    let mode = app.try_state::<ConfigState>().map(|s| s.snapshot().auto_resize).unwrap_or_default();
    if let Err(e) = crate::auto_resize::apply(&window, mode, height) {
        tracing::warn!(?e, "apply_auto_resize failed");
    }
}
```

Register in `src-tauri/src/lib.rs:31-43` invoke handler.

### Frontend measurement — `src/App.svelte`

- Wrap the existing `<header>` + `<main>` in a single `<div bind:this={widgetRoot}>` (the `.widget` root, lines 135–144).
- Use a `ResizeObserver` on `widgetRoot` to detect content-size changes; observed `contentRect` won't fire when the *window* resizes if the content's intrinsic size is unchanged (per Plan-agent validation). Avoid reading `window.innerHeight` anywhere in the measurement path.
- Compute `desired = header.offsetHeight + sessionList.scrollHeight + paddings`. Use `scrollHeight` so the SessionList's `overflow-y: auto` doesn't cap the natural height — no CSS change needed.
- Debounce 50 ms; dedup against `lastSentHeight` (skip if `Math.abs(measured - lastSent) < 1`).
- Only invoke when `config.auto_resize !== 'none'`.
- Svelte `$effect` watches `config.auto_resize`. On transition (incl. initial mount), force a measurement so the snap happens.

### External-edit hot-reload — `src-tauri/src/config_watcher.rs`

The watcher's `apply_config_to_window` (lines 85–98) currently syncs `always_on_top` and `window_position`. Add a branch: if `prior.auto_resize != cfg.auto_resize`, emit `config_updated` so the frontend `$effect` picks it up and re-measures. (No need to call `apply` from Rust — the frontend will drive it.)

### Files to modify

- `src-tauri/src/config.rs` — `AutoResize` enum + Config field + Default + schema test.
- `src-tauri/src/tray.rs` — submenu, three CheckMenuItem handles in `TrayHandles`, `select_auto_resize_mode` handler, IDs.
- `src-tauri/src/auto_resize.rs` (new) — `apply` + `clear_constraints`.
- `src-tauri/src/commands.rs` — `apply_auto_resize` command.
- `src-tauri/src/lib.rs` — register `mod auto_resize;` and the new command.
- `src-tauri/src/config_watcher.rs` — auto_resize change branch.
- `src/App.svelte` — ResizeObserver wiring + invoke + `$effect`.
- `src/lib/api.ts` — `applyAutoResize(height: number)` wrapper.
- `src/lib/types.ts` — `AutoResize` union type.

## Verification

1. `bash scripts/deploy.sh` — clean build, app launches.
2. Right-click tray → "Auto resize" → confirm three radio-style entries, "None" initially checked.
3. Select "Up". Window snaps; bottom edge unchanged from before, top moved up to fit content.
4. Trigger session add (interact with a Claude Code instance, or rely on the dev seed). Window grows upward — bottom anchored.
5. Trigger session remove. Window shrinks upward — bottom anchored.
6. Try dragging the bottom window edge. Should not move (height locked). Drag a side edge — width changes.
7. Switch to "Down". Window snaps with top fixed; mirror behavior on add/remove.
8. Switch to "None". Constraints cleared; vertical drag works again.
9. Edit `config.json` externally to flip `auto_resize` → window reacts.
10. Multi-monitor sanity: move window to a non-primary monitor, then trigger Up growth large enough to hit the top — window should clamp to that monitor's top, not primary's.
11. Restart with mode = Up. Window should fit current content with bottom near where it was at last close (the saved `window_position` is the bottom-anchored Up position).

## Risks / known limitations

- **Persisted position with content drift**: when `save_window_position` + Up are both enabled, restart with a different session count restores the saved Y — meaning the bottom edge won't necessarily match the last session. Acceptable; can be revisited later.
- **No `Resized` event handler** today, so user-driven horizontal resizes don't update the persisted width until close. Existing behavior — out of scope here.
- **Feedback-loop guard relies on measuring the content element only.** If a future change adds something to the measurement path that depends on `window.innerHeight`, the dedup will need to be re-validated.
