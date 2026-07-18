import { create } from 'zustand'
import type { SftpAdhocParams } from '@/lib/sftp'
import { deleteSecret } from '@/lib/vault'

export type SessionStatus = 'connecting' | 'connected' | 'closed' | 'error'
export type SplitDirection = 'horizontal' | 'vertical'

export interface PaneSession {
  id: string
  profileId: string
  title: string
  status: SessionStatus
  reconnectNonce: number
  sshId?: string | null
  // Font-size offset applied on top of the resolved appearance, per pane.
  zoom?: number
}

export type TabView =
  | { kind: 'welcome' }
  | { kind: 'settings' }
  | { kind: 'import' }
  | { kind: 'profile-editor'; profileId: string | null }
  | { kind: 'group-editor'; groupId: string | null }

export interface SessionTab {
  id: string
  kind: 'session'
  sessionIds: string[]
  direction: SplitDirection
  activePaneId: string
  broadcast?: boolean
}

export interface ViewTab {
  id: string
  kind: 'view'
  view: TabView
  title: string
}

export interface LocalTab {
  id: string
  kind: 'local'
  title: string
  program: string | null
}

export interface VncTab {
  id: string
  kind: 'vnc'
  title: string
  host: string
  port: number
  secretId: string | null
}

export interface SftpTab {
  id: string
  kind: 'sftp'
  title: string
  profileId: string | null
  adhoc: SftpAdhocParams | null
}

export interface FtpTab {
  id: string
  kind: 'ftp'
  title: string
  host: string
  port: number
  username: string
  secretId: string | null
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
}

export interface S3Tab {
  id: string
  kind: 's3'
  title: string
  profileId: string
  bucket: string | null
}

export type Tab = SessionTab | ViewTab | LocalTab | VncTab | SftpTab | FtpTab | S3Tab

export function tabSecretId(t: Tab): string | null {
  if (t.kind === 'ftp' || t.kind === 'vnc') return t.secretId
  if (t.kind === 'sftp') return t.adhoc?.secretId ?? null
  return null
}

function viewKey(v: TabView): string {
  if (v.kind === 'profile-editor') return `profile-editor:${v.profileId ?? 'new'}`
  if (v.kind === 'group-editor') return `group-editor:${v.groupId ?? 'new'}`
  return v.kind
}

interface SessionState {
  tabs: Tab[]
  sessions: Record<string, PaneSession>
  activeTabId: string | null
  openTab: (session: PaneSession) => void
  openView: (view: TabView, title: string) => void
  openLocalShell: (program?: string | null, title?: string) => void
  openVnc: (host: string, port: number, secretId: string | null) => void
  openSftp: (profileId: string, title: string) => void
  openSftpAdhoc: (params: SftpAdhocParams) => void
  openFtp: (params: {
    host: string
    port: number
    username: string
    secretId: string | null
    secure: boolean
    allowInvalidCert: boolean
    ignoreHostname: boolean
  }) => void
  openS3: (profileId: string, bucket: string | null, title: string) => void
  duplicateTab: (tabId: string) => void
  setActiveTab: (tabId: string) => void
  setActivePane: (tabId: string, sessionId: string) => void
  splitPane: (tabId: string, sessionId: string, direction: SplitDirection) => void
  closePane: (tabId: string, sessionId: string) => void
  removeTab: (tabId: string) => void
  setStatus: (sessionId: string, status: SessionStatus) => void
  setSshId: (sessionId: string, sshId: string) => void
  reconnect: (sessionId: string) => void
  toggleBroadcast: (tabId: string) => void
  setZoom: (sessionId: string, zoom: number) => void
  restoreTabs: (snapshot: {
    tabs: Tab[]
    sessions: Record<string, PaneSession>
    activeTabId: string | null
  }) => void
}

export const useSessionStore = create<SessionState>()((set, get) => ({
  tabs: [],
  sessions: {},
  activeTabId: null,

  openTab: (session) => {
    const tab: SessionTab = {
      id: crypto.randomUUID(),
      kind: 'session',
      sessionIds: [session.id],
      direction: 'horizontal',
      activePaneId: session.id,
    }
    set({
      tabs: [...get().tabs, tab],
      sessions: { ...get().sessions, [session.id]: session },
      activeTabId: tab.id,
    })
  },

  openView: (view, title) => {
    const key = viewKey(view)
    const existing = get().tabs.find((t) => t.kind === 'view' && viewKey(t.view) === key)
    if (existing) {
      set({ activeTabId: existing.id })
      return
    }
    const tab: ViewTab = { id: crypto.randomUUID(), kind: 'view', view, title }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openLocalShell: (program = null, title = 'Local shell') => {
    const tab: LocalTab = { id: crypto.randomUUID(), kind: 'local', title, program }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openVnc: (host, port, secretId) => {
    const tab: VncTab = {
      id: crypto.randomUUID(),
      kind: 'vnc',
      title: `${host}:${port}`,
      host,
      port,
      secretId,
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openSftp: (profileId, title) => {
    const tab: SftpTab = { id: crypto.randomUUID(), kind: 'sftp', title, profileId, adhoc: null }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openSftpAdhoc: (params) => {
    const tab: SftpTab = {
      id: crypto.randomUUID(),
      kind: 'sftp',
      title: `${params.host}:${params.port}`,
      profileId: null,
      adhoc: params,
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openFtp: ({ host, port, username, secretId, secure, allowInvalidCert, ignoreHostname }) => {
    const tab: FtpTab = {
      id: crypto.randomUUID(),
      kind: 'ftp',
      title: `${host}:${port}`,
      host,
      port,
      username,
      secretId,
      secure,
      allowInvalidCert,
      ignoreHostname,
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openS3: (profileId, bucket, title) => {
    const tab: S3Tab = { id: crypto.randomUUID(), kind: 's3', title, profileId, bucket }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  duplicateTab: (tabId) => {
    const tab = get().tabs.find((t) => t.id === tabId)
    if (!tab) return
    if (tab.kind === 'local') {
      get().openLocalShell(tab.program, tab.title)
    } else if (tab.kind === 'sftp') {
      if (tab.adhoc) get().openSftpAdhoc(tab.adhoc)
      else if (tab.profileId) get().openSftp(tab.profileId, tab.title)
    } else if (tab.kind === 'ftp') {
      get().openFtp(tab)
    } else if (tab.kind === 's3') {
      get().openS3(tab.profileId, tab.bucket, tab.title)
    } else if (tab.kind === 'session') {
      const src = get().sessions[tab.activePaneId]
      if (!src) return
      get().openTab({
        id: crypto.randomUUID(),
        profileId: src.profileId,
        title: src.title,
        status: 'connecting',
        reconnectNonce: 0,
      })
    }
  },

  setActiveTab: (tabId) => set({ activeTabId: tabId }),

  setActivePane: (tabId, sessionId) =>
    set({
      tabs: get().tabs.map((t) =>
        t.id === tabId && t.kind === 'session' ? { ...t, activePaneId: sessionId } : t,
      ),
    }),

  splitPane: (tabId, sessionId, direction) => {
    const src = get().sessions[sessionId]
    if (!src) return
    const newId = crypto.randomUUID()
    const newSession: PaneSession = {
      id: newId,
      profileId: src.profileId,
      title: src.title,
      status: 'connecting',
      reconnectNonce: 0,
    }
    set({
      sessions: { ...get().sessions, [newId]: newSession },
      tabs: get().tabs.map((t) => {
        if (t.id !== tabId || t.kind !== 'session') return t
        const at = t.sessionIds.indexOf(sessionId)
        const sessionIds = [...t.sessionIds]
        sessionIds.splice(at + 1, 0, newId)
        return { ...t, sessionIds, direction, activePaneId: newId }
      }),
    })
  },

  closePane: (tabId, sessionId) => {
    const tab = get().tabs.find((t) => t.id === tabId)
    if (tab?.kind !== 'session') return
    const sessions = { ...get().sessions }
    delete sessions[sessionId]
    const remaining = tab.sessionIds.filter((id) => id !== sessionId)
    if (remaining.length === 0) {
      const tabs = get().tabs.filter((t) => t.id !== tabId)
      set({
        tabs,
        sessions,
        activeTabId: get().activeTabId === tabId ? (tabs.at(-1)?.id ?? null) : get().activeTabId,
      })
      return
    }
    set({
      sessions,
      tabs: get().tabs.map((t) =>
        t.id === tabId && t.kind === 'session'
          ? {
              ...t,
              sessionIds: remaining,
              activePaneId: t.activePaneId === sessionId ? remaining[0] : t.activePaneId,
            }
          : t,
      ),
    })
  },

  removeTab: (tabId) => {
    const tab = get().tabs.find((t) => t.id === tabId)
    if (!tab) return
    const sessions = { ...get().sessions }
    if (tab.kind === 'session') for (const id of tab.sessionIds) delete sessions[id]
    const tabs = get().tabs.filter((t) => t.id !== tabId)
    // Drop the ad-hoc credential with its last tab; a duplicated tab still references it.
    const secretId = tabSecretId(tab)
    if (secretId && !tabs.some((t) => tabSecretId(t) === secretId)) {
      deleteSecret(secretId).catch(() => {})
    }
    set({
      tabs,
      sessions,
      activeTabId: get().activeTabId === tabId ? (tabs.at(-1)?.id ?? null) : get().activeTabId,
    })
  },

  setStatus: (sessionId, status) => {
    const prev = get().sessions[sessionId]
    if (!prev) return
    set({ sessions: { ...get().sessions, [sessionId]: { ...prev, status } } })
  },

  setSshId: (sessionId, sshId) => {
    const prev = get().sessions[sessionId]
    if (!prev) return
    set({ sessions: { ...get().sessions, [sessionId]: { ...prev, sshId } } })
  },

  reconnect: (sessionId) => {
    const prev = get().sessions[sessionId]
    if (!prev) return
    set({
      sessions: {
        ...get().sessions,
        [sessionId]: { ...prev, status: 'connecting', reconnectNonce: prev.reconnectNonce + 1 },
      },
    })
  },

  // Panes come back as 'connecting' with no ssh id, so PaneView's connect effect redials.
  restoreTabs: ({ tabs, sessions, activeTabId }) => {
    const revived: Record<string, PaneSession> = {}
    for (const [id, s] of Object.entries(sessions)) {
      revived[id] = { ...s, status: 'connecting', sshId: null, reconnectNonce: 0 }
    }
    set({ tabs, sessions: revived, activeTabId: activeTabId ?? tabs.at(-1)?.id ?? null })
  },

  toggleBroadcast: (tabId) =>
    set({
      tabs: get().tabs.map((t) =>
        t.id === tabId && t.kind === 'session' ? { ...t, broadcast: !t.broadcast } : t,
      ),
    }),

  setZoom: (sessionId, zoom) => {
    const prev = get().sessions[sessionId]
    if (!prev) return
    set({
      sessions: {
        ...get().sessions,
        [sessionId]: { ...prev, zoom: Math.max(-6, Math.min(12, zoom)) },
      },
    })
  },
}))
