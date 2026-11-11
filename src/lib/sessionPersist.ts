import type { AdhocParams } from '@/lib/sftp'
import { deleteSecret } from '@/lib/vault'
import {
  idle,
  type Pane,
  type PanesTab,
  type SessionStatus,
  type Tab,
  tabSecretIds,
} from '@/stores/sessionStore'

// 0.5.0 reads `wisp.session` and can't read 0.6.0's tabs, so 0.6.0 keeps its own key and
// takes the old one over only to convert it.
const KEY = 'wisp.tabs'
const OLD_KEY = 'wisp.session'

export interface SessionSnapshot {
  tabs: Tab[]
  panes: Record<string, Pane>
  activeTabId: string | null
}

export function saveSnapshot(state: SessionSnapshot): void {
  // Broadcast is a deliberate, dangerous mode - it must never come back silently enabled.
  const tabs = state.tabs.map((t) =>
    t.kind === 'panes' && t.broadcast ? { ...t, broadcast: false } : t,
  )
  const keep = new Set(tabs.flatMap((t) => (t.kind === 'panes' ? t.paneIds : [])))
  const panes: Record<string, Pane> = {}
  for (const [id, p] of Object.entries(state.panes)) {
    if (!keep.has(id)) continue
    panes[id] = idle(p)
  }
  const activeTabId = tabs.some((t) => t.id === state.activeTabId) ? state.activeTabId : null
  try {
    localStorage.setItem(KEY, JSON.stringify({ v: 2, tabs, panes, activeTabId }))
  } catch {
    // a full or unavailable localStorage must never break the session
  }
}

// Tab shapes saved by 0.5.0 and the first 0.6.0 checkpoints, one tab per terminal row or
// file browser. Profile ids became connection ids when profiles moved into hosts.
interface OldPaneSession {
  id: string
  profileId: string
  title: string
  adhoc?: AdhocParams
  zoom?: number
}
type OldTab =
  | {
      id: string
      kind: 'session'
      sessionIds: string[]
      direction: PanesTab['direction']
      activePaneId: string
    }
  | {
      id: string
      kind: 'local'
      title: string
      program: string | null
      startFolder?: string | null
      zoom?: number
    }
  | {
      id: string
      kind: 'sftp'
      title: string
      profileId: string | null
      sftpProfileId: string | null
      adhoc: AdhocParams | null
    }
  | {
      id: string
      kind: 'ftp'
      title: string
      connectionId?: string | null
      host: string
      port: number
      username: string
      secretId: string | null
      secure: boolean
      allowInvalidCert: boolean
      ignoreHostname: boolean
    }
  | { id: string; kind: 's3'; title: string; profileId: string; bucket: string | null }
  | { id: string; kind: 'view' }
  | {
      id: string
      kind: 'vnc'
      title: string
      host: string
      port: number
      username?: string | null
      secretId?: string | null
      profileId?: string | null
    }
  | {
      id: string
      kind: 'rdp'
      title: string
      host: string
      port: number
      username?: string
      domain?: string | null
      secretId?: string | null
      profileId?: string | null
      keyboardLayout?: number | null
    }
  | { id: string; kind: 'panes' }

const base = { status: 'connecting' as SessionStatus, reconnectNonce: 0 }

function convertOld(tabs: OldTab[], sessions: Record<string, OldPaneSession>): SessionSnapshot {
  const panes: Record<string, Pane> = {}
  const out: Tab[] = []
  const single = (tabId: string, pane: Pane) => {
    panes[pane.id] = pane
    out.push({
      id: tabId,
      kind: 'panes',
      paneIds: [pane.id],
      direction: 'horizontal',
      activePaneId: pane.id,
    })
  }
  for (const t of tabs) {
    switch (t.kind) {
      case 'session': {
        const paneIds = t.sessionIds.filter((id) => sessions[id])
        if (paneIds.length === 0) break
        for (const id of paneIds) {
          const s = sessions[id]
          panes[id] = {
            ...base,
            id,
            kind: 'ssh',
            title: s.title,
            zoom: s.zoom,
            connectionId: s.profileId || null,
            adhoc: s.adhoc,
            sshId: null,
          }
        }
        out.push({
          id: t.id,
          kind: 'panes',
          paneIds,
          direction: t.direction,
          activePaneId: paneIds.includes(t.activePaneId) ? t.activePaneId : paneIds[0],
        })
        break
      }
      case 'local':
        single(t.id, {
          ...base,
          id: crypto.randomUUID(),
          kind: 'local',
          title: t.title,
          zoom: t.zoom,
          connectionId: null,
          program: t.program,
          startFolder: t.startFolder ?? null,
        })
        break
      case 'sftp':
        single(t.id, {
          ...base,
          id: crypto.randomUUID(),
          kind: 'sftp',
          title: t.title,
          connectionId: t.sftpProfileId,
          sshConnectionId: t.profileId,
          viaSession: null,
          adhoc: t.adhoc,
        })
        break
      case 'ftp':
        single(t.id, {
          ...base,
          id: crypto.randomUUID(),
          kind: 'ftp',
          title: t.title,
          connectionId: t.connectionId ?? null,
          host: t.host,
          port: t.port,
          username: t.username,
          secretId: t.secretId,
          secure: t.secure,
          allowInvalidCert: t.allowInvalidCert,
          ignoreHostname: t.ignoreHostname,
        })
        break
      case 's3':
        single(t.id, {
          ...base,
          id: crypto.randomUUID(),
          kind: 's3',
          title: t.title,
          connectionId: t.profileId,
          bucket: t.bucket,
        })
        break
      // 0.5.0 also saved view tabs (settings, editors); those pages now live in the Hosts tab.
      case 'view':
        break
      // Fields an older version didn't save yet read as empty.
      case 'vnc':
        out.push({
          id: t.id,
          kind: 'vnc',
          title: t.title,
          connectionId: t.profileId ?? null,
          once: t.profileId
            ? null
            : {
                host: t.host,
                port: t.port,
                username: t.username ?? null,
                secretId: t.secretId ?? null,
              },
        })
        break
      case 'rdp':
        out.push({
          id: t.id,
          kind: 'rdp',
          title: t.title,
          connectionId: t.profileId ?? null,
          once: t.profileId
            ? null
            : {
                host: t.host,
                port: t.port,
                username: t.username ?? '',
                domain: t.domain ?? null,
                secretId: t.secretId ?? null,
                keyboardLayout: t.keyboardLayout ?? null,
              },
        })
        break
      case 'panes':
        break
    }
  }
  return { tabs: out, panes, activeTabId: null }
}

const PANE_KINDS = new Set(['ssh', 'local', 'sftp', 'ftp', 's3'])

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object'

// Whatever doesn't have the shape the views need is left out, so a damaged snapshot can't
// break every start.
function wellFormed(snap: SessionSnapshot): SessionSnapshot {
  const panes: Record<string, Pane> = {}
  for (const [id, p] of Object.entries(isObject(snap.panes) ? snap.panes : {})) {
    if (isObject(p) && p.id === id && PANE_KINDS.has(p.kind as string)) panes[id] = p as Pane
  }
  const tabs: Tab[] = []
  for (const t of Array.isArray(snap.tabs) ? (snap.tabs as unknown[]) : []) {
    if (!isObject(t) || typeof t.id !== 'string') continue
    if (t.kind === 'panes') {
      const ids = Array.isArray(t.paneIds) ? t.paneIds : []
      const paneIds = ids.filter((id): id is string => typeof id === 'string' && !!panes[id])
      if (paneIds.length === 0) continue
      tabs.push({
        id: t.id,
        kind: 'panes',
        paneIds,
        direction: t.direction === 'vertical' ? 'vertical' : 'horizontal',
        activePaneId: paneIds.includes(t.activePaneId as string)
          ? (t.activePaneId as string)
          : paneIds[0],
        broadcast: false,
      })
    } else if (
      (t.kind === 'vnc' || t.kind === 'rdp') &&
      (typeof t.connectionId === 'string' || isObject(t.once))
    ) {
      tabs.push(t as unknown as Tab)
    }
  }
  const activeTabId = tabs.some((t) => t.id === snap.activeTabId) ? snap.activeTabId : null
  return { tabs, panes, activeTabId }
}

function read(key: string): SessionSnapshot | null {
  const raw = localStorage.getItem(key)
  if (!raw) return null
  const parsed = JSON.parse(raw)
  if (!isObject(parsed) || !Array.isArray(parsed.tabs)) return null
  const activeTabId = typeof parsed.activeTabId === 'string' ? parsed.activeTabId : null
  const snap: SessionSnapshot =
    parsed.v === 2
      ? { tabs: parsed.tabs, panes: (parsed.panes ?? {}) as Record<string, Pane>, activeTabId }
      : {
          ...convertOld(
            (parsed.tabs as unknown[]).filter(isObject) as OldTab[],
            isObject(parsed.sessions) ? (parsed.sessions as Record<string, OldPaneSession>) : {},
          ),
          activeTabId,
        }
  return wellFormed(snap)
}

const owned = (snap: SessionSnapshot) =>
  new Set(snap.tabs.flatMap((t) => tabSecretIds(t, snap.panes)))

export function loadSnapshot(): SessionSnapshot | null {
  try {
    const old = read(OLD_KEY)
    let snap = read(KEY)
    if (old) {
      // Still there means 0.5.0 wrote it after this version last ran: it is the newer one.
      if (snap) {
        const keep = owned(old)
        for (const id of owned(snap)) if (!keep.has(id)) deleteSecret(id).catch(() => {})
      }
      snap = old
      localStorage.setItem(KEY, JSON.stringify({ v: 2, ...snap }))
      localStorage.removeItem(OLD_KEY)
    }
    return snap && snap.tabs.length > 0 ? snap : null
  } catch {
    return null
  }
}

function savedConnections(p: Pane): (string | null)[] {
  if (p.kind === 'sftp') return [p.connectionId, p.sshConnectionId]
  return [p.connectionId]
}

// Panes and tabs whose saved connection was deleted since the snapshot can't reconnect;
// connect-once ones carry their own login and stay.
export function dropMissing(snap: SessionSnapshot, connectionIds: Set<string>): SessionSnapshot {
  const gone = (id: string | null) => id !== null && !connectionIds.has(id)
  const panes: Record<string, Pane> = {}
  for (const [id, p] of Object.entries(snap.panes)) {
    if (!savedConnections(p).some(gone)) panes[id] = p
  }
  const tabs: Tab[] = []
  for (const t of snap.tabs) {
    if (t.kind !== 'panes') {
      if (!gone(t.connectionId)) tabs.push(t)
      continue
    }
    const paneIds = t.paneIds.filter((id) => panes[id])
    if (paneIds.length === 0) continue
    tabs.push({
      ...t,
      paneIds,
      activePaneId: paneIds.includes(t.activePaneId) ? t.activePaneId : paneIds[0],
    })
  }
  const activeTabId = tabs.some((t) => t.id === snap.activeTabId) ? snap.activeTabId : null
  return { tabs, panes, activeTabId }
}

export function clearSnapshot(): void {
  try {
    localStorage.removeItem(KEY)
    localStorage.removeItem(OLD_KEY)
  } catch {
    // ignore
  }
}
