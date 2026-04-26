# Fix: limit dashboard looks frozen after idle periods

## Context

User report: "limit dashboard stops updating when there are no claude sessions running, though seeing remaining limits would be still helpful."

What `widget.jsonl` actually shows during the user's observed "frozen" state:

- No `ai_agent_dashboard_lib::usage_limits` log lines at all for *hours* — well past the configured 10-minute poll interval.
- When a new `SessionStart` hook POST finally arrives (e.g. `08:34:57` on Apr 24), a `usage poll success` fires ~3 seconds later, even though the last poll was hours ago (`04:44:58`).

Interpretation: Windows' power management (efficiency mode / background-process throttling) suspends the Tauri process while the widget window is occluded and no external events arrive. `tokio::time::sleep` cannot fire in a suspended process. As soon as anything wakes the process — an HTTP event on port 9077, the user un-minimizing the window, a tray menu click — the overdue sleep resolves immediately and the poller runs. But *just looking at the widget* currently doesn't wake the poller, so the user sees whatever snapshot was last written hours ago. The existing 10-minute tick cannot help here because the process isn't executing the tick at all.

A second, smaller issue: when the 5h bucket has 0% utilization, Anthropic's `/api/oauth/usage` returns `resets_at: null`. The current frontend renders that as `-:--:--` in the time cap — visually indistinguishable from "broken/no data" status. Not the primary complaint but worth fixing in the same pass since both touch `LimitBar.svelte`.

Outcome we want:
1. When the user's widget becomes visible after an idle period, a fresh poll fires promptly (subject to Anthropic's rate limits) so the bars reflect current reality rather than a stale snapshot.
2. `resets_at: null` shows an informative `IDLE` state, not dashes.

## Approach

### 1. Wake the poller on widget visibility — `Notify`-driven + frontend trigger

**Why this shape:** the frontend is the authority on "user just brought the widget forward"; the backend owns the rate-limit guard on the Anthropic endpoint. Waking the *existing* poll loop (rather than spawning a parallel task) keeps the single-writer invariant on `UsageLimitsState` and lets `Notify::notify_one` coalesce rapid wake signals. Because any invoke from the frontend also implicitly wakes the Tauri process, calling a Tauri command is exactly the trigger needed — we don't need a separate "unsuspend" mechanism.

**Backend — `src-tauri/src/usage_limits.rs`:**

- Promote `MIN_POLL_SECS` to `pub const MIN_POLL_SECS: u64 = 60;` so `request_refresh` can reuse the floor.
- Add a `wake: Arc<Notify>` field to `UsageLimitsState`; constructor seeds `Arc::new(Notify::new())`. Imports: `use tokio::sync::Notify; use std::sync::Arc;`.
- Replace `tokio::time::sleep(Duration::from_secs(secs)).await` in `UsageLimitsPoller::spawn` with `tokio::select!` over `sleep(secs)` and `state.wake.notified()`. Either branch falls through to the next loop iteration → immediate `poll_once`.
- New method on `UsageLimitsState`:
  ```rust
  pub fn request_refresh(&self) -> bool {
      let age_ms = now_ms() - self.snapshot().updated;
      if age_ms < (MIN_POLL_SECS * 1000) as i64 {
          return false; // rate-limited: existing data is still fresh
      }
      self.wake.notify_one();
      true
  }
  ```
  Returns a bool so the command and tests can observe whether a poll was scheduled.

**Backend — `src-tauri/src/commands.rs`:**

- Add command:
  ```rust
  #[tauri::command]
  pub fn refresh_usage_limits(state: State<UsageLimitsState>) -> bool {
      state.request_refresh()
  }
  ```

**Backend — `src-tauri/src/lib.rs`:**

- Register `commands::refresh_usage_limits` in the `invoke_handler![…]` list.

**Frontend — `src/lib/api.ts`:**

- Export `refreshUsageLimits = (): Promise<boolean> => invoke('refresh_usage_limits')`.

**Frontend — `src/App.svelte`:**

- In `onMount`, register a `visibilitychange` listener. When `document.visibilityState === 'visible'`, call `refreshUsageLimits().catch(…)`. Unregister in the teardown returned from `onMount`.
- Deliberately not adding a `focus` listener — WebView2's `visibilitychange` fires on un-minimize / un-occlude, which is exactly the moment we care about. Doubling listeners risks double-fires without benefit.

### 2. LimitBar visual: `IDLE` text when `resets_at` is null

File: `src/lib/components/LimitBar.svelte`

- `timeText` (~line 31): when `status === 'ok'` and `bucket.resets_at === null`, render the right cap as `IDLE` (uppercase, matches the existing state-pill convention in `stateLabel` — `IDLE / WORK / WAIT / DONE / ERROR`). Keep `-:--:--` / `--:--` only for the genuine no-data/error states so dashes continue to signal "broken/unavailable" and `IDLE` signals "healthy, no active window".
- `buildTooltip` (~line 42): when status is ok and `b.resets_at === null`, append `No active window` instead of the `Resets …` line, still followed by `updated {ago}`.
- No CSS change — `.cap-right` already has `min-width: calc(7ch + 10px)` so four characters fit and remain centered by the flex parent.

### 3. Rate-limit guard

Lives in a single place: `UsageLimitsState::request_refresh`. Floor is `MIN_POLL_SECS = 60` measured against `snapshot().updated` (the last poll attempt's timestamp, updated by every code path in `poll_once` including error branches). `wake.notify_one()` coalesces multiple rapid notifies into at most one extra poll. Frontend cannot bypass this — it only observes a `bool` and cannot reach into the state directly.

## Files to modify

- `src-tauri/src/usage_limits.rs` — `MIN_POLL_SECS` visibility, `wake` field, `select!` loop, `request_refresh` method, unit test.
- `src-tauri/src/commands.rs` — `refresh_usage_limits` command.
- `src-tauri/src/lib.rs` — register command in invoke handler.
- `src/lib/api.ts` — export `refreshUsageLimits`.
- `src/lib/components/LimitBar.svelte` — `timeText` and `buildTooltip` handle `resets_at: null` as `idle`.
- `src/App.svelte` — visibility listener.

## Verification

- **Wake-on-visibility (end-to-end):** run the app, alt-tab to another window for 2+ minutes, alt-tab back. Confirm a `usage poll success` log line in `widget.jsonl` within ~1s of regaining visibility. This is the primary behavior change the user asked for — verify against the actual observed symptom.
- **Rate-limit guard (unit test):** seed `UsageLimitsState` with `updated = now_ms() - 30_000` → `request_refresh()` returns `false` (no notify). Re-seed with `updated = now_ms() - 61_000` → returns `true`. Rapid visibility toggles should not produce polls closer than 60s apart in the log.
- **Visual `IDLE` fix:** easiest to trigger by not using Claude for 5+ hours so the 5h bucket lands at 0% / `resets_at: null`, then opening the widget. Cap right-hand text reads `IDLE` (not `-:--:--`); tooltip shows `No active window` + `updated Ns ago`. Alternatively add a one-off `#[cfg(debug_assertions)]` dev helper to inject a synthetic `UsageLimits { five_hour: Some(LimitBucket { utilization: 0.0, resets_at: None }), … }` for faster iteration.
