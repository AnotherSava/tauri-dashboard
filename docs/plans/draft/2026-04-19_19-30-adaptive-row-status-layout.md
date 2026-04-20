# Adaptive row layout: float status column to vertical middle when label is short

## Status

**Draft / postponed.** Do not start until Stage 1 (UI skeleton) ships and the v1 basic layout has been in use for a while. Revisit if the "always-on-top row" feels visually heavy.

## Context

In the current `SessionItem.svelte` layout, each row has:

- A leading dot (vertically centered)
- A content area with two lines stacked: top line = `id · STATE · time · tokens`, bottom line = sticky label

When the label is long it fills its line and the layout looks balanced. When the label is short (or empty — see `idle` sessions with no task yet), the bottom-right corner below `STATE/time/tokens` is empty, and the row reads unevenly: the status metadata is forced up to the top line, making a lot of whitespace.

The user wants: **if the label's right edge falls to the left of the `STATE` pill, move `STATE/time/tokens` off the top line and center them vertically between the two lines** — matching the leading dot's vertical-centering. When the label is long, keep the current "spread" layout.

Must respond to window resize (the widget is resizable).

## Why this isn't a trivial CSS change

- CSS cannot conditionally reposition sibling element B based on the *rendered content width* of sibling A. Container queries respond to the container's own size, not to its children's natural content size.
- Flex/grid can wrap and reorder, but not the exact "swap to vertical-center" behavior we want.
- A heuristic CSS-only approach using container queries (switch at a fixed row-width breakpoint) doesn't satisfy the spec — a short label in a wide window still gets the spread layout.

## Proposed approach

Per-row `ResizeObserver` + text-width measurement:

1. In `SessionItem.svelte`, add an `$effect` block that sets up a `ResizeObserver` on the row element.
2. Also measure the label's natural (un-truncated) text width. Two reasonable paths:
   - **Simplest**: put the label text inside a `position: absolute; visibility: hidden; white-space: nowrap` probe sibling, read `probe.offsetWidth`, and use that.
   - **Cleaner**: use a `canvas.measureText` with the same font as the label element to compute width without DOM.
3. Read the row's width and the status-column width (`getBoundingClientRect`). Compute available label space = row width − dot − id-min-width − gaps.
4. If `labelWidth ≤ availableWidthBeforeStatus`, toggle the row to `compact` mode (status column absolute-positioned at right, vertically centered). Otherwise, `spread` mode (current layout).
5. Run the check on:
   - ResizeObserver callback (row size changed)
   - Whenever the session prop changes (new label)
   - Once on mount (after font load — guard with `document.fonts.ready`)

### CSS shape

```css
.row                 { display: grid; grid-template-columns: auto 1fr auto; column-gap: 10px; align-items: center; }
.row .dot            { grid-column: 1; }
.row .content        { grid-column: 2; display: flex; flex-direction: column; min-width: 0; }
.row .status-group   { grid-column: 3; }

/* Default: spread — status-group inline with id on top line. Achieved by hoisting
   status-group into the content's top line via DOM reorder, OR by default layout
   before the class toggle. */

.row.compact .status-group { align-self: center; }
.row.spread  .status-group { align-self: start; /* inline with top line */ }
```

(The exact DOM structure needs tweaking — likely the status-group becomes a sibling of the content column, and only its vertical alignment changes.)

### Edge cases to test

- Very long ID + short label → does the status still fit?
- Empty label (idle session) → compact mode, single visual line
- Font not loaded at mount time → re-measure after `document.fonts.ready`
- Window resize causing scrollbar to appear → re-measure
- HiDPI / DPR changes → ResizeObserver should fire
- Many rows (100+) → perf: throttle measurements, share one observer

## Effort estimate

1–2 hours of implementation + tuning the threshold math + regression testing across row contents and window widths.

## Fallback / interim behavior

Option 1 from the plan discussion — **always center the status column** — was considered. It's a 5-minute CSS change and is visually consistent, just slightly different from the original Electron widget. If adaptive layout slips, we can land option 1 instead.

## Files to touch (when picked up)

- `src/lib/components/SessionItem.svelte` — structural change + ResizeObserver + class toggle
- Potentially `src/lib/utils/textWidth.ts` — if we go with canvas-based measurement
