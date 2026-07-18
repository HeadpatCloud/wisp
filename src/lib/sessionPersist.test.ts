import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ deleteSecret: vi.fn().mockResolvedValue(undefined) }))

import { deleteSecret } from '@/lib/vault'
import { type PaneSession, type Tab, useSessionStore } from '@/stores/sessionStore'
import { clearSnapshot, loadSnapshot, saveSnapshot } from './sessionPersist'

const pane = (id: string): PaneSession => ({
  id,
  profileId: 'p1',
  title: 'web-01',
  status: 'connected',
  reconnectNonce: 3,
  sshId: 'live-ssh-id',
})

const sessionTab: Tab = {
  id: 't1',
  kind: 'session',
  sessionIds: ['s1'],
  direction: 'horizontal',
  activePaneId: 's1',
}

beforeEach(() => {
  vi.clearAllMocks()
  clearSnapshot()
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
})

test('round-trips session tabs and resets live connection state', () => {
  saveSnapshot({ tabs: [sessionTab], sessions: { s1: pane('s1') }, activeTabId: 't1' })
  const snap = loadSnapshot()
  expect(snap?.tabs).toHaveLength(1)
  expect(snap?.sessions.s1.status).toBe('connecting')
  expect(snap?.sessions.s1.sshId).toBeNull()
  expect(snap?.sessions.s1.reconnectNonce).toBe(0)
  expect(snap?.sessions.s1.profileId).toBe('p1')
})

// Credentials live in the vault now, so ad-hoc tabs are restorable and only carry an id.
test('keeps every connection tab and stores only vault references', () => {
  const tabs: Tab[] = [
    sessionTab,
    {
      id: 't2',
      kind: 'ftp',
      title: 'ftp',
      host: 'h',
      port: 21,
      username: 'u',
      secretId: 'vault-1',
      secure: true,
      allowInvalidCert: false,
      ignoreHostname: false,
    },
    { id: 't3', kind: 'vnc', title: 'vnc', host: 'h', port: 5900, secretId: 'vault-2' },
    {
      id: 't4',
      kind: 'sftp',
      title: 'adhoc',
      profileId: null,
      adhoc: {
        host: 'h',
        port: 22,
        username: 'u',
        authMethod: 'password',
        keyPath: '',
        secretId: 'vault-3',
      },
    },
    { id: 't5', kind: 'sftp', title: 'saved', profileId: 'p1', adhoc: null },
    { id: 't6', kind: 'view', view: { kind: 'settings' }, title: 'Settings' },
  ]
  saveSnapshot({ tabs, sessions: { s1: pane('s1') }, activeTabId: 't2' })

  const snap = loadSnapshot()
  expect(snap?.tabs.map((t) => t.id)).toEqual(['t1', 't2', 't3', 't4', 't5'])
  expect(snap?.activeTabId).toBe('t2')
  // Ids only - a snapshot must never be able to leak a usable credential.
  const raw = JSON.stringify(snap)
  expect(raw).toContain('vault-1')
  expect(raw).not.toContain('password":"')
})

test('drops the active id when its tab was not persisted', () => {
  const view: Tab = { id: 'v1', kind: 'view', view: { kind: 'settings' }, title: 'Settings' }
  saveSnapshot({ tabs: [sessionTab, view], sessions: { s1: pane('s1') }, activeTabId: 'v1' })
  expect(loadSnapshot()?.activeTabId).toBeNull()
})

test('never restores a tab with broadcast still armed', () => {
  saveSnapshot({
    tabs: [{ ...sessionTab, broadcast: true }],
    sessions: { s1: pane('s1') },
    activeTabId: 't1',
  })
  const tab = loadSnapshot()?.tabs[0]
  expect(tab?.kind === 'session' && tab.broadcast).toBe(false)
})

// The whole restart path: live store -> disk -> fresh store.
test('round-trips the real store and arms every pane for reconnect', () => {
  useSessionStore.setState({
    tabs: [
      sessionTab,
      { id: 't2', kind: 'vnc', title: 'vnc', host: 'h', port: 5900, secretId: 'vault-2' },
    ],
    sessions: { s1: pane('s1') },
    activeTabId: 't1',
  })
  saveSnapshot(useSessionStore.getState())

  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
  const snap = loadSnapshot()
  if (!snap) throw new Error('nothing persisted')
  useSessionStore.getState().restoreTabs(snap)

  const st = useSessionStore.getState()
  expect(st.tabs.map((t) => t.id)).toEqual(['t1', 't2'])
  expect(st.activeTabId).toBe('t1')
  // Live fields must not survive, or the pane would render as already connected.
  expect(st.sessions.s1.status).toBe('connecting')
  expect(st.sessions.s1.sshId).toBeNull()
  expect(st.sessions.s1.profileId).toBe('p1')
})

test('closing a tab drops its vault secret, but not while a duplicate holds it', async () => {
  const vnc = (id: string): Tab => ({
    id,
    kind: 'vnc',
    title: 'vnc',
    host: 'h',
    port: 5900,
    secretId: 'shared',
  })
  useSessionStore.setState({ tabs: [vnc('a'), vnc('b')], sessions: {}, activeTabId: 'a' })

  useSessionStore.getState().removeTab('a')
  expect(deleteSecret).not.toHaveBeenCalled()

  useSessionStore.getState().removeTab('b')
  expect(deleteSecret).toHaveBeenCalledWith('shared')
})

test('returns null when nothing restorable was stored', () => {
  saveSnapshot({
    tabs: [{ id: 'v', kind: 'view', view: { kind: 'welcome' }, title: 'Welcome' }],
    sessions: {},
    activeTabId: 'v',
  })
  expect(loadSnapshot()).toBeNull()
})
