<script lang="ts">
  import { onMount, tick } from 'svelte'
  import SessionList from './lib/components/SessionList.svelte'
  import LimitBar from './lib/components/LimitBar.svelte'
  import {
    getConfig,
    getSessions,
    getUsageLimits,
    hideWindow,
    onConfigUpdated,
    onSessionsUpdated,
    onUsageLimitsUpdated,
    showWindow,
  } from './lib/api'
  import type { AgentSession, Config, UsageLimits } from './lib/types'

  let sessions = $state<AgentSession[]>([])
  let config = $state<Config | null>(null)
  let usage = $state<UsageLimits | null>(null)
  let now = $state(Date.now())

  onMount(() => {
    let unlistenSessions: (() => void) | undefined
    let unlistenConfig: (() => void) | undefined
    let unlistenUsage: (() => void) | undefined

    ;(async () => {
      try {
        config = await getConfig()
        sessions = await getSessions()
        usage = await getUsageLimits()
        unlistenSessions = await onSessionsUpdated((s) => (sessions = s))
        unlistenConfig = await onConfigUpdated((c) => (config = c))
        unlistenUsage = await onUsageLimitsUpdated((u) => (usage = u))
      } catch (err) {
        console.error('failed to initialize', err)
      } finally {
        await tick()
        try {
          await showWindow()
        } catch (err) {
          console.error('failed to reveal window', err)
        }
      }
    })()

    const tickId = setInterval(() => (now = Date.now()), 1000)
    return () => {
      clearInterval(tickId)
      unlistenSessions?.()
      unlistenConfig?.()
      unlistenUsage?.()
    }
  })

  function onHide() {
    hideWindow().catch((err) => console.error('hide failed', err))
  }
</script>

<div class="widget">
  <header data-tauri-drag-region>
    <span class="title" data-tauri-drag-region>AI AGENTS</span>
    <div class="limits">
      <LimitBar
        bucket={usage?.five_hour ?? null}
        status={usage?.status ?? 'unavailable'}
        updated={usage?.updated ?? 0}
        {now}
        format="hm"
      />
      <LimitBar
        bucket={usage?.seven_day ?? null}
        status={usage?.status ?? 'unavailable'}
        updated={usage?.updated ?? 0}
        {now}
        format="dhm"
      />
    </div>
    <button class="hide-btn" onclick={onHide} aria-label="Hide to tray" title="Hide to tray">×</button>
  </header>
  {#if config}
    <SessionList {sessions} {config} {now} />
  {/if}
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: transparent;
    overflow: hidden;
    font-family: system-ui, 'Segoe UI', Roboto, sans-serif;
  }
  :global(*) {
    box-sizing: border-box;
  }
  .widget {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background: #1c1c1e;
    color: #d6d6d6;
    user-select: none;
    -webkit-user-select: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 4px 12px;
    background: #17171a;
    border-bottom: 1px solid #2a2a2d;
    cursor: grab;
  }
  header:active {
    cursor: grabbing;
  }
  .title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.6px;
    color: #8a8a8e;
    flex-shrink: 0;
  }
  .limits {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 18px;
    flex: 1;
    min-width: 0;
  }
  .limits > :global(*) {
    flex: 1 1 0;
    min-width: 0;
  }
  .hide-btn {
    background: transparent;
    border: 0;
    padding: 0 6px;
    color: #8a8a8e;
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    border-radius: 3px;
    opacity: 0;
    transition: opacity 120ms ease, background 120ms ease, color 120ms ease;
    margin-left: auto;
    flex-shrink: 0;
  }
  header:hover .hide-btn {
    opacity: 1;
  }
  .hide-btn:hover {
    background: #2a2a2d;
    color: #e8e8ea;
  }
</style>
