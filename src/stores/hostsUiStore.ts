import { create } from 'zustand'
import type { ConnectionType } from '@/lib/hosts'
import { useSessionStore } from '@/stores/sessionStore'

export type HostsPage =
  | 'hosts'
  | 'settings'
  | 'import'
  | 'export'
  | 'new'
  | 'edit'
  | 'sshImport'
  | 'tunnels'
  | 'keys'
  | 'knownHosts'

export interface NewConnectionPrefill {
  type?: ConnectionType
  address?: string
  port?: number
  user?: string
  bucket?: string
  // Adds the connection to this host.
  hostId?: string
}
export type TypeFilter = 'all' | 'terminal' | 'files' | 'desktop'
export type SettingsSection = 'appearance' | 'terminal' | 'tags' | 'keyboard' | 'general'

interface HostsUiState {
  page: HostsPage
  importPath: string | null
  newConnection: NewConnectionPrefill | null
  editHostId: string | null
  editConnectionId: string | null
  selectedHostId: string | null
  renamingHostId: string | null
  // null is All hosts.
  folderId: string | null
  tagId: string | null
  typeFilter: TypeFilter
  query: string
  openPage: (page: HostsPage, importPath?: string) => void
  openNewConnection: (prefill?: NewConnectionPrefill) => void
  // A connection id opens the editor with that connection's row expanded.
  openEditor: (hostId: string, connectionId?: string) => void
  select: (id: string | null) => void
  setRenaming: (id: string | null) => void
  setFolder: (id: string | null) => void
  setTag: (id: string | null) => void
  setType: (t: TypeFilter) => void
  setQuery: (q: string) => void
  settingsSection: SettingsSection
  openSettings: (section: SettingsSection) => void
  // The host list beside session tabs, folded to an icon strip. Not saved.
  hostListCollapsed: boolean
  toggleHostList: () => void
}

export const useHostsUiStore = create<HostsUiState>()((set) => ({
  page: 'hosts',
  importPath: null,
  newConnection: null,
  editHostId: null,
  editConnectionId: null,
  selectedHostId: null,
  renamingHostId: null,
  folderId: null,
  tagId: null,
  typeFilter: 'all',
  query: '',
  hostListCollapsed: false,
  settingsSection: 'appearance',
  openSettings: (settingsSection) => {
    set({ page: 'settings', settingsSection })
    useSessionStore.getState().setActiveTab(null)
  },
  // The pages live in the Hosts tab, so opening one brings that tab to the front.
  openPage: (page, importPath) => {
    set(importPath === undefined ? { page } : { page, importPath })
    useSessionStore.getState().setActiveTab(null)
  },
  openNewConnection: (prefill = {}) => {
    set({ page: 'new', newConnection: prefill })
    useSessionStore.getState().setActiveTab(null)
  },
  openEditor: (editHostId, editConnectionId) => {
    set({ page: 'edit', editHostId, editConnectionId: editConnectionId ?? null })
    useSessionStore.getState().setActiveTab(null)
  },
  select: (selectedHostId) => set({ selectedHostId }),
  setRenaming: (renamingHostId) => set({ renamingHostId }),
  setFolder: (folderId) => set({ folderId, tagId: null, page: 'hosts' }),
  setTag: (tagId) => set({ tagId, folderId: null, page: 'hosts' }),
  setType: (typeFilter) => set({ typeFilter }),
  setQuery: (query) => set({ query }),
  toggleHostList: () => set((s) => ({ hostListCollapsed: !s.hostListCollapsed })),
}))
