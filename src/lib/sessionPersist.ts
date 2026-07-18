import type { PaneSession, Tab } from '@/stores/sessionStore'

const KEY = 'wisp.session'

export interface SessionSnapshot {
  tabs: Tab[]
  sessions: Record<string, PaneSession>
  activeTabId: string | null
}

// Every credential now lives in the vault and tabs only carry its id, so the only thing
// not worth restoring is transient view tabs (settings, editors).
function restorable(t: Tab): boolean {
  return t.kind !== 'view'
}

export function saveSnapshot(state: SessionSnapshot): void {
  // Broadcast is a deliberate, dangerous mode - it must never come back silently enabled.
  const tabs = state.tabs
    .filter(restorable)
    .map((t) => (t.kind === 'session' && t.broadcast ? { ...t, broadcast: false } : t))
  const keep = new Set(tabs.flatMap((t) => (t.kind === 'session' ? t.sessionIds : [])))
  const sessions: Record<string, PaneSession> = {}
  for (const [id, s] of Object.entries(state.sessions)) {
    if (keep.has(id)) sessions[id] = { ...s, status: 'connecting', reconnectNonce: 0, sshId: null }
  }
  const activeTabId = tabs.some((t) => t.id === state.activeTabId) ? state.activeTabId : null
  try {
    localStorage.setItem(KEY, JSON.stringify({ tabs, sessions, activeTabId }))
  } catch {
    // a full or unavailable localStorage must never break the session
  }
}

export function loadSnapshot(): SessionSnapshot | null {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as SessionSnapshot
    if (!Array.isArray(parsed.tabs) || parsed.tabs.length === 0) return null
    return parsed
  } catch {
    return null
  }
}

export function clearSnapshot(): void {
  try {
    localStorage.removeItem(KEY)
  } catch {
    // ignore
  }
}
