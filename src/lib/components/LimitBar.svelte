<script lang="ts">
  import type { LimitBucket, UsageStatus } from '../types'
  import { formatCompactRemaining } from '../types'

  interface Props {
    bucket: LimitBucket | null
    status: UsageStatus
    updated: number
    now: number
    format: 'hm' | 'dhm'
  }

  let { bucket, status, updated, now, format }: Props = $props()

  const SEGMENT_PX = 8
  const PITCH_PX = SEGMENT_PX + 1

  let trackEl: HTMLDivElement | undefined = $state()
  let trackWidth = $state(0)

  $effect(() => {
    if (!trackEl) return
    const obs = new ResizeObserver((entries) => {
      for (const entry of entries) {
        trackWidth = entry.contentRect.width
      }
    })
    obs.observe(trackEl)
    return () => obs.disconnect()
  })

  const hasData = $derived(bucket !== null)
  const utilization = $derived(hasData && bucket ? bucket.utilization : 0)
  const totalSegments = $derived(Math.max(0, Math.floor(trackWidth / PITCH_PX)))
  const filledSegments = $derived(
    utilization > 0 && totalSegments > 0
      ? Math.max(
          1,
          Math.min(totalSegments, Math.round(utilization * totalSegments)),
        )
      : 0,
  )
  const fillWidthPx = $derived(filledSegments * PITCH_PX)
  const percentText = $derived(
    !hasData || !bucket
      ? '--%'
      : status === 'ok'
        ? `${Math.round(bucket.utilization * 100)}%`
        : 'NO DATA',
  )
  const timeText = $derived(
    hasData && bucket
      ? formatCompactRemaining(bucket.resets_at - now, format)
      : formatCompactRemaining(null, format),
  )
  const fillColor = $derived(
    utilization >= 0.85 ? '#b91c1c' : utilization >= 0.5 ? '#b45309' : '#047857',
  )
  const labelText = $derived(format === 'hm' ? '5h limit' : '7d limit')
  const tooltip = $derived(buildTooltip(status, bucket, updated, now, labelText))

  function buildTooltip(
    s: UsageStatus,
    b: LimitBucket | null,
    u: number,
    n: number,
    label: string,
  ): string {
    const resets = b ? `Resets ${formatResetTime(b.resets_at)}` : null
    const lines: string[] = [label]
    if (s === 'unavailable') lines.push('Sign in via Claude Code to enable')
    else if (s === 'auth_expired') lines.push('Token expired — run Claude Code to refresh')
    else if (s === 'network_error') {
      if (resets) lines.push(resets)
      lines.push(`Anthropic API unreachable — last try ${formatAgo(n - u)}`)
    } else if (s === 'ok' && b && resets) {
      lines.push(resets, `updated ${formatAgo(n - u)}`)
    }
    return lines.join('\n')
  }

  function formatAgo(ms: number): string {
    if (ms < 0) return 'just now'
    const s = Math.floor(ms / 1000)
    if (s < 60) return `${s}s ago`
    const m = Math.floor(s / 60)
    if (m < 60) return `${m}m ago`
    const h = Math.floor(m / 60)
    return `${h}h ago`
  }

  function formatResetTime(ms: number): string {
    return new Date(ms).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    })
  }
</script>

<div class="bar" title={tooltip}>
  <div class="track" bind:this={trackEl}>
    <div class="fill" style:width="{fillWidthPx}px" style:--fill-color={fillColor}></div>
  </div>
  <span class="pct">{percentText}</span>
  <span class="time">{timeText}</span>
</div>

<style>
  .bar {
    position: relative;
    height: 16px;
    display: flex;
    align-items: center;
  }
  .track {
    position: relative;
    flex: 1;
    height: 16px;
    min-width: 0;
    background-color: #17171a;
    background-image: linear-gradient(to right, #45454a 8px, transparent 8px);
    background-size: 9px 100%;
    background-repeat: repeat-x;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 3px;
    overflow: hidden;
  }
  .fill {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    background-image: linear-gradient(to right, var(--fill-color) 8px, transparent 8px);
    background-size: 9px 100%;
    background-repeat: repeat-x;
    transition: width 280ms ease;
  }
  .pct, .time {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    font-family: ui-monospace, Consolas, monospace;
    font-variant-numeric: tabular-nums;
    font-size: 10px;
    font-weight: 700;
    color: #f5f5f7;
    background: rgba(0, 0, 0, 0.65);
    padding: 1px 3px;
    border-radius: 2px;
    pointer-events: none;
    line-height: 1;
  }
  .pct {
    left: 2px;
  }
  .time {
    right: 2px;
  }
</style>
