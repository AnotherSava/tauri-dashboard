import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { AgentSession, Config, UsageLimits } from './types'

export function getSessions(): Promise<AgentSession[]> {
  return invoke<AgentSession[]>('get_sessions')
}

export function getConfig(): Promise<Config> {
  return invoke<Config>('get_config')
}

export function getUsageLimits(): Promise<UsageLimits> {
  return invoke<UsageLimits>('get_usage_limits')
}

export function hideWindow(): Promise<void> {
  return invoke('hide_window')
}

export function showWindow(): Promise<void> {
  return invoke('show_window')
}

export function toggleWindow(): Promise<void> {
  return invoke('toggle_window')
}

export function quitApp(): Promise<void> {
  return invoke('quit_app')
}

export function removeSession(id: string): Promise<void> {
  return invoke('remove_session', { id })
}

export function onSessionsUpdated(
  handler: (sessions: AgentSession[]) => void,
): Promise<UnlistenFn> {
  return listen<AgentSession[]>('sessions_updated', (evt) => handler(evt.payload))
}

export function onConfigUpdated(
  handler: (config: Config) => void,
): Promise<UnlistenFn> {
  return listen<Config>('config_updated', (evt) => handler(evt.payload))
}

export function onUsageLimitsUpdated(
  handler: (usage: UsageLimits) => void,
): Promise<UnlistenFn> {
  return listen<UsageLimits>('usage_limits_updated', (evt) => handler(evt.payload))
}
