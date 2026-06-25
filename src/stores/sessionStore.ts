import { create } from 'zustand'

export type SessionStatus = 'connecting' | 'connected' | 'closed' | 'error'
export type SplitDirection = 'horizontal' | 'vertical'

export interface PaneSession {
  id: string
  profileId: string
  title: string
  status: SessionStatus
  reconnectNonce: number
  sshId?: string | null
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
  password: string
}

export interface SftpTab {
  id: string
  kind: 'sftp'
  title: string
  profileId: string
}

export interface FtpTab {
  id: string
  kind: 'ftp'
  title: string
  host: string
  port: number
  username: string
  password: string
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
}

export type Tab = SessionTab | ViewTab | LocalTab | VncTab | SftpTab | FtpTab

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
  openVnc: (host: string, port: number, password: string) => void
  openSftp: (profileId: string, title: string) => void
  openFtp: (params: {
    host: string
    port: number
    username: string
    password: string
    secure: boolean
    allowInvalidCert: boolean
    ignoreHostname: boolean
  }) => void
  duplicateTab: (tabId: string) => void
  setActiveTab: (tabId: string) => void
  setActivePane: (tabId: string, sessionId: string) => void
  splitPane: (tabId: string, sessionId: string, direction: SplitDirection) => void
  closePane: (tabId: string, sessionId: string) => void
  removeTab: (tabId: string) => void
  setStatus: (sessionId: string, status: SessionStatus) => void
  setSshId: (sessionId: string, sshId: string) => void
  reconnect: (sessionId: string) => void
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

  openVnc: (host, port, password) => {
    const tab: VncTab = {
      id: crypto.randomUUID(),
      kind: 'vnc',
      title: `${host}:${port}`,
      host,
      port,
      password,
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openSftp: (profileId, title) => {
    const tab: SftpTab = { id: crypto.randomUUID(), kind: 'sftp', title, profileId }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  openFtp: ({ host, port, username, password, secure, allowInvalidCert, ignoreHostname }) => {
    const tab: FtpTab = {
      id: crypto.randomUUID(),
      kind: 'ftp',
      title: `${host}:${port}`,
      host,
      port,
      username,
      password,
      secure,
      allowInvalidCert,
      ignoreHostname,
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id })
  },

  duplicateTab: (tabId) => {
    const tab = get().tabs.find((t) => t.id === tabId)
    if (!tab) return
    if (tab.kind === 'local') {
      get().openLocalShell(tab.program, tab.title)
    } else if (tab.kind === 'sftp') {
      get().openSftp(tab.profileId, tab.title)
    } else if (tab.kind === 'ftp') {
      get().openFtp(tab)
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
}))
