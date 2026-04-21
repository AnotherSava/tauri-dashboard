# Add hide-to-tray button in window header

## Context

With `skipTaskbar: true` the widget has no taskbar entry, and with `decorations: false` it has no native title-bar close button. The tray icon is now the only way to hide the window — fine if you're reaching for the tray already, but awkward if your cursor is in the widget. Add an in-header button that hides the window (the app keeps running in tray) so users have a fast in-place dismissal.

## Change

`src/App.svelte`:

1. Import `hideWindow` from `./lib/api` (wrapper already exists, backed by the `hide_window` Tauri command in `src-tauri/src/commands.rs:17`).
2. Add `onHide()` handler: `hideWindow().catch(err => console.error('hide failed', err))`.
3. In the header JSX, add a `<button class="hide-btn">` as the second child of `<header>`. With `justify-content: space-between` on the header flex container, the title flows to the **left** and the button sits at the **top-right corner** — the conventional Windows close-button position. The button deliberately omits `data-tauri-drag-region` so clicks don't start a window drag.
4. **Hover-reveal** (matching the row-remove pattern in `SessionItem.svelte`): the button is `opacity: 0` by default and fades in when the pointer is over the header (`header:hover .hide-btn { opacity: 1 }`), with a 120ms ease transition. Keeps the minimalist widget uncluttered when idle; same mental model as the red × over session timers.
5. Style: transparent background, muted gray `×`, subtle `#2a2a2d` hover fill on the button itself for press feedback.

## Verification

1. `deploy` (or `npm run tauri dev`).
2. Widget header shows just "AI AGENTS" on the left — no visible × at rest.
3. Move pointer into the header — × fades in at the top-right.
4. Click the × — window hides, process still running (visible in tray).
5. Left-click tray icon — window reappears at the same position.
6. Move pointer off the header — × fades back out.
7. Grab-drag the header title area — still drags the window (button is not a drag region but the rest of the header is).
8. Hovering the × itself shows the "Hide to tray" tooltip and the subtle hover fill.
