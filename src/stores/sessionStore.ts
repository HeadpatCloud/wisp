import { create } from 'zustand'
import type { AdhocParams } from '@/lib/sftp'
import { deleteSecret } from '@/lib/vault'

export type SessionStatus = 'connecting' | 'connected' | 'closed' | 'error'
export type SplitDirection = 'horizontal' | 'vertical'

interface PaneBase {
  id: string
  title: string
  status: SessionStatus
  reconnectNonce: number
  // Font-size offset applied on top of the resolved appearance, per pane.
  zoom?: number
}

export interface SshPane extends PaneBase {
  kind: 'ssh'
  // Null for connect once, which carries its own login in `adhoc`.
  connectionId: string | null
  adhoc?: AdhocParams
  sshId?: string | null
}

export interface LocalPane extends PaneBase {
  kind: 'local'
  connectionId: string | null
  program: string | null
  startFolder: string | null
  localId?: string | null
}

export interface SftpPane extends PaneBase {
  kind: 'sftp'
  // A saved Files connection, resolved at connect time.
  connectionId: string | null
  // Files over a saved SSH connection, without a Files connection of its own.
  sshConnectionId: string | null
  // Files on a live terminal's own SSH session (sftp_for).
  viaSession: string | null
  adhoc: AdhocParams | null
}

export interface FtpPane extends PaneBase {
  kind: 'ftp'
  // A saved connection, read at connect time; host, port and user are then for display only.
  connectionId: string | null
  host: string
  port: number
  username: string
  secretId: string | null
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
}

export interface S3Pane extends PaneBase {
  kind: 's3'
  connectionId: string
  bucket: string | null
}

export type Pane = SshPane | LocalPane | SftpPane | FtpPane | S3Pane
export type PaneKind = Pane['kind']
type NewPane =
  | Omit<SshPane, keyof PaneBase>
  | Omit<LocalPane, keyof PaneBase>
  | Omit<SftpPane, keyof PaneBase>
  | Omit<FtpPane, keyof PaneBase>
  | Omit<S3Pane, keyof PaneBase>

// Terminals and file browsers share one tab kind: a row or column of panes.
export interface PanesTab {
  id: string
  kind: 'panes'
  paneIds: string[]
  direction: SplitDirection
  activePaneId: string
  broadcast?: boolean
}

// Connect once: what a remote desktop tab dials when it has no saved connection.
export interface VncTarget {
  host: string
  port: number
  username: string | null
  secretId: string | null
}

export interface RdpTarget {
  host: string
  port: number
  username: string
  domain: string | null
  secretId: string | null
  // Null picks the local layout.
  keyboardLayout: number | null
}

// A saved connection is read again at every connect, so its tab holds only the id.
export interface VncTab {
  id: string
  kind: 'vnc'
  title: string
  connectionId: string | null
  once: VncTarget | null
  status?: SessionStatus
}

export interface RdpTab {
  id: string
  kind: 'rdp'
  title: string
  connectionId: string | null
  once: RdpTarget | null
  status?: SessionStatus
}

export type Tab = PanesTab | VncTab | RdpTab

const adhocSecretIds = (p: AdhocParams) =>
  [p.secretId, ...p.keys.map((k) => k.secretId)].filter((id): id is string => !!id)

function paneSecretIds(p: Pane): string[] {
  if (p.kind === 'ssh' && p.adhoc) return adhocSecretIds(p.adhoc)
  if (p.kind === 'sftp' && p.adhoc) return adhocSecretIds(p.adhoc)
  if (p.kind === 'ftp' && !p.connectionId && p.secretId) return [p.secretId]
  return []
}

// Vault entries a tab owns outright, so closing it can clean them up. Secrets of a saved
// connection are excluded: the connection owns those and outlives the tab.
export function tabSecretIds(t: Tab, panes: Record<string, Pane> = {}): string[] {
  if (t.kind === 'vnc' || t.kind === 'rdp') return t.once?.secretId ? [t.once.secretId] : []
  return t.paneIds.flatMap((id) => (panes[id] ? paneSecretIds(panes[id]) : []))
}

function ownedSecretIds(tabs: Tab[], panes: Record<string, Pane>): Set<string> {
  return new Set(tabs.flatMap((t) => tabSecretIds(t, panes)))
}

// Ad-hoc credentials go with the last tab or pane that uses them; a duplicate keeps them alive.
function releaseSecrets(before: Set<string>, after: Set<string>) {
  for (const id of before) if (!after.has(id)) deleteSecret(id).catch(() => {})
}

function clampZoom(zoom: number): number {
  return Math.max(-6, Math.min(12, zoom))
}

// A pane as it starts: not connected, and holding no live session id.
export function idle(p: Pane): Pane {
  const base = { ...p, status: 'connecting' as const, reconnectNonce: 0 }
  if (base.kind === 'ssh') return { ...base, sshId: null }
  if (base.kind === 'local') return { ...base, localId: null }
  return base
}

const fresh = (p: Pane): Pane => ({ ...idle(p), id: crypto.randomUUID() })

interface OpenOptions {
  // Adds the pane to the active panes tab instead of a new tab.
  split?: SplitDirection
}

interface SessionState {
  tabs: Tab[]
  panes: Record<string, Pane>
  activeTabId: string | null
  // Where a split opened from the Hosts tab goes: the panes tab used last.
  lastPanesTabId: string | null
  openPane: (pane: NewPane & { title: string }, opts?: OpenOptions) => void
  openSsh: (connectionId: string, title: string, opts?: OpenOptions) => void
  openSshAdhoc: (params: AdhocParams, title?: string) => void
  openLocalShell: (
    program?: string | null,
    title?: string,
    startFolder?: string | null,
    connectionId?: string | null,
    opts?: OpenOptions,
  ) => void
  openVnc: (tab: { connectionId: string; title: string } | { once: VncTarget }) => void
  openRdp: (tab: { connectionId: string; title: string } | { once: RdpTarget }) => void
  // Files over a saved SSH connection.
  openSftp: (sshConnectionId: string, title: string, opts?: OpenOptions) => void
  // A saved Files connection.
  openSftpProfile: (connectionId: string, title: string, opts?: OpenOptions) => void
  openSftpAdhoc: (params: AdhocParams, title?: string) => void
  openFtp: (params: {
    host: string
    port: number
    username: string
    secretId: string | null
    secure: boolean
    allowInvalidCert: boolean
    ignoreHostname: boolean
  }) => void
  openFtpSaved: (
    connectionId: string,
    title: string,
    shown: { host: string; port: number; username: string; secure: boolean },
    opts?: OpenOptions,
  ) => void
  openS3: (connectionId: string, bucket: string | null, title: string, opts?: OpenOptions) => void
  duplicateTab: (tabId: string) => void
  // null is the Hosts tab.
  setActiveTab: (tabId: string | null) => void
  setActivePane: (tabId: string, paneId: string) => void
  splitPane: (tabId: string, paneId: string, direction: SplitDirection) => void
  // Opens files on a terminal pane's live SSH session, beside it.
  openFilesBeside: (tabId: string, paneId: string) => void
  closePane: (tabId: string, paneId: string) => void
  removeTab: (tabId: string) => void
  moveTab: (tabId: string, delta: number) => void
  moveTabTo: (tabId: string, index: number) => void
  // `before` puts the moved panes first (a drop on the left or top edge).
  mergeTab: (
    fromTabId: string,
    intoTabId: string,
    direction: SplitDirection,
    before?: boolean,
  ) => void
  setStatus: (paneId: string, status: SessionStatus) => void
  setTabStatus: (tabId: string, status: SessionStatus) => void
  setSshId: (paneId: string, sshId: string) => void
  setLocalId: (paneId: string, localId: string | null) => void
  reconnect: (paneId: string) => void
  toggleBroadcast: (tabId: string) => void
  setZoom: (paneId: string, zoom: number) => void
  restoreTabs: (snapshot: {
    tabs: Tab[]
    panes: Record<string, Pane>
    activeTabId: string | null
  }) => void
}

// Makes a tab active; a panes tab also becomes where a split from the Hosts tab goes.
function activate(tabs: Tab[], id: string | null) {
  return tabs.find((t) => t.id === id)?.kind === 'panes'
    ? { activeTabId: id, lastPanesTabId: id }
    : { activeTabId: id }
}

// Files opened on a closing terminal's session connect on their own from then on.
function releaseFiles(panes: Record<string, Pane>, closing: Pane[]) {
  const ended = new Set(closing.flatMap((p) => (p.kind === 'ssh' && p.sshId ? [p.sshId] : [])))
  for (const [id, p] of Object.entries(panes)) {
    if (p.kind === 'sftp' && p.viaSession && ended.has(p.viaSession)) {
      panes[id] = { ...p, viaSession: null }
    }
  }
}

export const useSessionStore = create<SessionState>()((set, get) => {
  const addTab = (tab: Tab) => {
    const tabs = [...get().tabs, tab]
    set({ tabs, ...activate(tabs, tab.id) })
  }

  return {
    tabs: [],
    panes: {},
    activeTabId: null,
    lastPanesTabId: null,

    openPane: (spec, opts = {}) => {
      const pane = {
        ...spec,
        id: crypto.randomUUID(),
        status: 'connecting',
        reconnectNonce: 0,
      } as Pane
      const tabs = get().tabs
      const target =
        tabs.find((t) => t.id === get().activeTabId && t.kind === 'panes') ??
        tabs.find((t) => t.id === get().lastPanesTabId) ??
        [...tabs].reverse().find((t) => t.kind === 'panes')
      if (opts.split && target?.kind === 'panes') {
        set({
          ...activate(tabs, target.id),
          panes: { ...get().panes, [pane.id]: pane },
          // A pane that joins doesn't silently start taking what is typed into the others.
          tabs: tabs.map((t) =>
            t.id === target.id && t.kind === 'panes'
              ? {
                  ...t,
                  paneIds: [...t.paneIds, pane.id],
                  direction: opts.split ?? t.direction,
                  activePaneId: pane.id,
                  broadcast: false,
                }
              : t,
          ),
        })
        return
      }
      const tab: PanesTab = {
        id: crypto.randomUUID(),
        kind: 'panes',
        paneIds: [pane.id],
        direction: 'horizontal',
        activePaneId: pane.id,
      }
      set({
        panes: { ...get().panes, [pane.id]: pane },
        tabs: [...get().tabs, tab],
        activeTabId: tab.id,
        lastPanesTabId: tab.id,
      })
    },

    openSsh: (connectionId, title, opts) =>
      get().openPane({ kind: 'ssh', connectionId, title }, opts),

    openSshAdhoc: (params, title) =>
      get().openPane({
        kind: 'ssh',
        connectionId: null,
        adhoc: params,
        title: title ?? `${params.username ? `${params.username}@` : ''}${params.host}`,
      }),

    openLocalShell: (
      program = null,
      title = 'Local shell',
      startFolder = null,
      connectionId = null,
      opts,
    ) => get().openPane({ kind: 'local', program, title, startFolder, connectionId }, opts),

    openVnc: (t) =>
      addTab(
        'once' in t
          ? {
              id: crypto.randomUUID(),
              kind: 'vnc',
              title: `${t.once.host}:${t.once.port}`,
              connectionId: null,
              once: t.once,
            }
          : {
              id: crypto.randomUUID(),
              kind: 'vnc',
              title: t.title,
              connectionId: t.connectionId,
              once: null,
            },
      ),

    openRdp: (t) =>
      addTab(
        'once' in t
          ? {
              id: crypto.randomUUID(),
              kind: 'rdp',
              title: `${t.once.host}:${t.once.port}`,
              connectionId: null,
              once: t.once,
            }
          : {
              id: crypto.randomUUID(),
              kind: 'rdp',
              title: t.title,
              connectionId: t.connectionId,
              once: null,
            },
      ),

    openSftp: (sshConnectionId, title, opts) =>
      get().openPane(
        { kind: 'sftp', title, connectionId: null, sshConnectionId, viaSession: null, adhoc: null },
        opts,
      ),

    openSftpProfile: (connectionId, title, opts) =>
      get().openPane(
        { kind: 'sftp', title, connectionId, sshConnectionId: null, viaSession: null, adhoc: null },
        opts,
      ),

    openSftpAdhoc: (params, title) =>
      get().openPane({
        kind: 'sftp',
        title: title ?? `${params.host}:${params.port}`,
        connectionId: null,
        sshConnectionId: null,
        viaSession: null,
        adhoc: params,
      }),

    openFtp: (params) =>
      get().openPane({
        kind: 'ftp',
        title: `${params.host}:${params.port}`,
        connectionId: null,
        ...params,
      }),

    openFtpSaved: (connectionId, title, shown, opts) =>
      get().openPane(
        {
          kind: 'ftp',
          title,
          connectionId,
          host: shown.host,
          port: shown.port,
          username: shown.username,
          secure: shown.secure,
          secretId: null,
          allowInvalidCert: false,
          ignoreHostname: false,
        },
        opts,
      ),

    openS3: (connectionId, bucket, title, opts) =>
      get().openPane({ kind: 's3', connectionId, bucket, title }, opts),

    duplicateTab: (tabId) => {
      const tab = get().tabs.find((t) => t.id === tabId)
      if (!tab) return
      if (tab.kind === 'vnc' || tab.kind === 'rdp') {
        addTab({ ...tab, id: crypto.randomUUID(), status: undefined })
        return
      }
      const src = get().panes[tab.activePaneId]
      if (!src) return
      const copy = fresh(src)
      const next: PanesTab = {
        id: crypto.randomUUID(),
        kind: 'panes',
        paneIds: [copy.id],
        direction: 'horizontal',
        activePaneId: copy.id,
      }
      const tabs = [...get().tabs, next]
      set({ panes: { ...get().panes, [copy.id]: copy }, tabs, ...activate(tabs, next.id) })
    },

    setActiveTab: (tabId) => set(activate(get().tabs, tabId)),

    setActivePane: (tabId, paneId) =>
      set({
        tabs: get().tabs.map((t) =>
          t.id === tabId && t.kind === 'panes' ? { ...t, activePaneId: paneId } : t,
        ),
      }),

    splitPane: (tabId, paneId, direction) => {
      const src = get().panes[paneId]
      if (!src) return
      const copy = fresh(src)
      set({
        panes: { ...get().panes, [copy.id]: copy },
        tabs: get().tabs.map((t) => {
          if (t.id !== tabId || t.kind !== 'panes') return t
          const paneIds = [...t.paneIds]
          paneIds.splice(paneIds.indexOf(paneId) + 1, 0, copy.id)
          return { ...t, paneIds, direction, activePaneId: copy.id }
        }),
      })
    },

    openFilesBeside: (tabId, paneId) => {
      const src = get().panes[paneId]
      if (src?.kind !== 'ssh' || !src.sshId) return
      const files: SftpPane = {
        id: crypto.randomUUID(),
        kind: 'sftp',
        title: src.title,
        status: 'connecting',
        reconnectNonce: 0,
        connectionId: null,
        sshConnectionId: src.connectionId,
        viaSession: src.sshId,
        // Connect once has no saved connection, so the files pane keeps the login for when
        // it has to connect on its own.
        adhoc: src.adhoc ?? null,
      }
      set({
        panes: { ...get().panes, [files.id]: files },
        tabs: get().tabs.map((t) => {
          if (t.id !== tabId || t.kind !== 'panes') return t
          const paneIds = [...t.paneIds]
          paneIds.splice(paneIds.indexOf(paneId) + 1, 0, files.id)
          return { ...t, paneIds, activePaneId: files.id }
        }),
      })
    },

    closePane: (tabId, paneId) => {
      const tab = get().tabs.find((t) => t.id === tabId)
      if (tab?.kind !== 'panes') return
      const panes = { ...get().panes }
      delete panes[paneId]
      const closing = get().panes[paneId]
      if (closing) releaseFiles(panes, [closing])
      const remaining = tab.paneIds.filter((id) => id !== paneId)
      const before = ownedSecretIds(get().tabs, get().panes)
      const tabs =
        remaining.length === 0
          ? get().tabs.filter((t) => t.id !== tabId)
          : get().tabs.map((t) =>
              t.id === tabId && t.kind === 'panes'
                ? {
                    ...t,
                    paneIds: remaining,
                    activePaneId: t.activePaneId === paneId ? remaining[0] : t.activePaneId,
                  }
                : t,
            )
      releaseSecrets(before, ownedSecretIds(tabs, panes))
      const activeTabId =
        remaining.length === 0 && get().activeTabId === tabId
          ? (tabs.at(-1)?.id ?? null)
          : get().activeTabId
      set({ tabs, panes, ...activate(tabs, activeTabId) })
    },

    removeTab: (tabId) => {
      const tab = get().tabs.find((t) => t.id === tabId)
      if (!tab) return
      const panes = { ...get().panes }
      if (tab.kind === 'panes') {
        for (const id of tab.paneIds) delete panes[id]
        releaseFiles(
          panes,
          tab.paneIds.map((id) => get().panes[id]).filter((p): p is Pane => !!p),
        )
      }
      const tabs = get().tabs.filter((t) => t.id !== tabId)
      releaseSecrets(ownedSecretIds(get().tabs, get().panes), ownedSecretIds(tabs, panes))
      const activeTabId =
        get().activeTabId === tabId ? (tabs.at(-1)?.id ?? null) : get().activeTabId
      set({ tabs, panes, ...activate(tabs, activeTabId) })
    },

    moveTab: (tabId, delta) => {
      const at = get().tabs.findIndex((t) => t.id === tabId)
      if (at < 0) return
      get().moveTabTo(tabId, at + delta)
    },

    moveTabTo: (tabId, index) => {
      const tabs = [...get().tabs]
      const at = tabs.findIndex((t) => t.id === tabId)
      if (at < 0) return
      const [tab] = tabs.splice(at, 1)
      tabs.splice(Math.max(0, Math.min(index, tabs.length)), 0, tab)
      set({ tabs })
    },

    // Dropping one panes tab on the edge of another's content joins them in one row or column.
    mergeTab: (fromTabId, intoTabId, direction, before = false) => {
      if (fromTabId === intoTabId) return
      const from = get().tabs.find((t) => t.id === fromTabId)
      const into = get().tabs.find((t) => t.id === intoTabId)
      if (from?.kind !== 'panes' || into?.kind !== 'panes') return
      set({
        tabs: get()
          .tabs.filter((t) => t.id !== fromTabId)
          .map((t) =>
            t.id === intoTabId && t.kind === 'panes'
              ? {
                  ...t,
                  paneIds: before
                    ? [...from.paneIds, ...t.paneIds]
                    : [...t.paneIds, ...from.paneIds],
                  direction,
                  activePaneId: from.activePaneId,
                  broadcast: false,
                }
              : t,
          ),
        ...activate(get().tabs, intoTabId),
      })
    },

    setStatus: (paneId, status) => {
      const prev = get().panes[paneId]
      if (!prev || prev.status === status) return
      set({ panes: { ...get().panes, [paneId]: { ...prev, status } } })
    },

    setTabStatus: (tabId, status) =>
      set({
        tabs: get().tabs.map((t) =>
          t.id === tabId && (t.kind === 'vnc' || t.kind === 'rdp') ? { ...t, status } : t,
        ),
      }),

    // Files opened on the terminal's session move to its new session after a reconnect.
    setSshId: (paneId, sshId) => {
      const prev = get().panes[paneId]
      if (prev?.kind !== 'ssh') return
      const panes: Record<string, Pane> = { ...get().panes, [paneId]: { ...prev, sshId } }
      if (prev.sshId) {
        for (const [id, p] of Object.entries(panes)) {
          if (p.kind === 'sftp' && p.viaSession === prev.sshId)
            panes[id] = { ...p, viaSession: sshId }
        }
      }
      set({ panes })
    },

    setLocalId: (paneId, localId) => {
      const prev = get().panes[paneId]
      if (prev?.kind !== 'local') return
      set({ panes: { ...get().panes, [paneId]: { ...prev, localId } } })
    },

    reconnect: (paneId) => {
      const prev = get().panes[paneId]
      if (!prev) return
      set({
        panes: {
          ...get().panes,
          [paneId]: { ...prev, status: 'connecting', reconnectNonce: prev.reconnectNonce + 1 },
        },
      })
    },

    // Panes come back as 'connecting' with no ssh id, so their connect effects redial.
    restoreTabs: ({ tabs, panes, activeTabId }) => {
      const revived: Record<string, Pane> = {}
      for (const [id, p] of Object.entries(panes)) {
        // A live terminal session doesn't survive a restart; a files pane's saved connection does.
        const pane = idle(p)
        revived[id] = pane.kind === 'sftp' ? { ...pane, viaSession: null } : pane
      }
      set({ tabs, panes: revived, ...activate(tabs, activeTabId ?? tabs.at(-1)?.id ?? null) })
    },

    toggleBroadcast: (tabId) =>
      set({
        tabs: get().tabs.map((t) =>
          t.id === tabId && t.kind === 'panes' ? { ...t, broadcast: !t.broadcast } : t,
        ),
      }),

    setZoom: (paneId, zoom) => {
      const prev = get().panes[paneId]
      if (!prev) return
      set({ panes: { ...get().panes, [paneId]: { ...prev, zoom: clampZoom(zoom) } } })
    },
  }
})
