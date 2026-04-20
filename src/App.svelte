<script lang="ts">
  import { onMount, tick } from 'svelte'
  import SessionList from './lib/components/SessionList.svelte'
  import {
    getConfig,
    getSessions,
    onConfigUpdated,
    onSessionsUpdated,
    showWindow,
  } from './lib/api'
  import type { AgentSession, Config } from './lib/types'

  let sessions = $state<AgentSession[]>([])
  let config = $state<Config | null>(null)
  let now = $state(Date.now())

  onMount(() => {
    let unlistenSessions: (() => void) | undefined
    let unlistenConfig: (() => void) | undefined

    ;(async () => {
      try {
        config = await getConfig()
        sessions = await getSessions()
        unlistenSessions = await onSessionsUpdated((s) => (sessions = s))
        unlistenConfig = await onConfigUpdated((c) => (config = c))
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
    }
  })
</script>

<div class="widget">
  <header data-tauri-drag-region>
    <span class="title" data-tauri-drag-region>AI AGENTS</span>
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
    padding: 6px 12px;
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
  }
</style>
