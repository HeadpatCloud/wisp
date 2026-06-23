import { beforeEach, expect, test } from 'vitest'
import type { SessionTab } from './sessionStore'
import { useSessionStore } from './sessionStore'

const session = (id: string, profileId = 'p1') => ({
  id,
  profileId,
  title: id,
  status: 'connecting' as const,
  reconnectNonce: 0,
})

beforeEach(() => useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null }))

test('openTab creates a single-pane tab and activates it', () => {
  useSessionStore.getState().openTab(session('s1'))
  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(1)
  expect(st.tabs[0]).toMatchObject({ kind: 'session', sessionIds: ['s1'] })
  expect(st.activeTabId).toBe(st.tabs[0].id)
})

test('openLocalShell adds an active local tab', () => {
  useSessionStore.getState().openLocalShell()
  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(1)
  expect(st.tabs[0]).toMatchObject({ kind: 'local', title: 'Local shell' })
  expect(st.activeTabId).toBe(st.tabs[0].id)
})

test('duplicateTab on a session opens a new session tab with the same profile', () => {
  useSessionStore.getState().openTab(session('s1', 'p9'))
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().duplicateTab(tabId)
  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(2)
  const newTab = st.tabs[1] as SessionTab
  expect(newTab.kind).toBe('session')
  expect(st.sessions[newTab.sessionIds[0]].profileId).toBe('p9')
  expect(st.activeTabId).toBe(newTab.id)
})

test('duplicateTab on a local tab opens another local tab', () => {
  useSessionStore.getState().openLocalShell()
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().duplicateTab(tabId)
  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(2)
  expect(st.tabs.every((t) => t.kind === 'local')).toBe(true)
})

test('splitPane inserts a new pane after the split one and focuses it', () => {
  useSessionStore.getState().openTab(session('s1'))
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().splitPane(tabId, 's1', 'vertical')
  const tab = useSessionStore.getState().tabs[0] as SessionTab
  expect(tab.sessionIds).toHaveLength(2)
  expect(tab.sessionIds[0]).toBe('s1')
  expect(tab.direction).toBe('vertical')
  expect(tab.activePaneId).toBe(tab.sessionIds[1])
  expect(Object.keys(useSessionStore.getState().sessions)).toHaveLength(2)
})

test('closePane removes one pane; closing the last removes the tab', () => {
  useSessionStore.getState().openTab(session('s1'))
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().splitPane(tabId, 's1', 'horizontal')
  const newId = (useSessionStore.getState().tabs[0] as SessionTab).sessionIds[1]
  useSessionStore.getState().closePane(tabId, newId)
  expect((useSessionStore.getState().tabs[0] as SessionTab).sessionIds).toEqual(['s1'])
  useSessionStore.getState().closePane(tabId, 's1')
  expect(useSessionStore.getState().tabs).toHaveLength(0)
  expect(useSessionStore.getState().activeTabId).toBeNull()
})

test('closePane reassigns focus when the active pane closes', () => {
  useSessionStore.getState().openTab(session('s1'))
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().splitPane(tabId, 's1', 'horizontal')
  const newId = (useSessionStore.getState().tabs[0] as SessionTab).activePaneId
  useSessionStore.getState().closePane(tabId, newId)
  expect((useSessionStore.getState().tabs[0] as SessionTab).activePaneId).toBe('s1')
})

test('reconnect bumps the nonce and sets connecting', () => {
  useSessionStore
    .getState()
    .openTab({ id: 's1', profileId: 'p1', title: 's1', status: 'closed', reconnectNonce: 0 })
  useSessionStore.getState().reconnect('s1')
  expect(useSessionStore.getState().sessions.s1.status).toBe('connecting')
  expect(useSessionStore.getState().sessions.s1.reconnectNonce).toBe(1)
})

test('setStatus + setSshId update one session; removeTab drops the tab and its sessions', () => {
  useSessionStore.getState().openTab(session('s1'))
  const tabId = useSessionStore.getState().tabs[0].id
  useSessionStore.getState().splitPane(tabId, 's1', 'horizontal')
  useSessionStore.getState().setStatus('s1', 'connected')
  useSessionStore.getState().setSshId('s1', 'rust-1')
  expect(useSessionStore.getState().sessions.s1.status).toBe('connected')
  expect(useSessionStore.getState().sessions.s1.sshId).toBe('rust-1')
  useSessionStore.getState().removeTab(tabId)
  expect(useSessionStore.getState().tabs).toHaveLength(0)
  expect(Object.keys(useSessionStore.getState().sessions)).toHaveLength(0)
})

test('openView opens a view tab and activates it', () => {
  useSessionStore.getState().openView({ kind: 'settings' }, 'Settings')
  const { tabs, activeTabId } = useSessionStore.getState()
  expect(tabs).toHaveLength(1)
  expect(tabs[0]).toMatchObject({ kind: 'view', title: 'Settings' })
  expect(activeTabId).toBe(tabs[0].id)
})

test('openView focuses an existing singleton instead of duplicating', () => {
  const s = useSessionStore.getState()
  s.openView({ kind: 'settings' }, 'Settings')
  const firstId = useSessionStore.getState().tabs[0].id
  s.openView({ kind: 'settings' }, 'Settings')
  expect(useSessionStore.getState().tabs).toHaveLength(1)
  expect(useSessionStore.getState().activeTabId).toBe(firstId)
})

test('openView dedupes editors by target id', () => {
  const s = useSessionStore.getState()
  s.openView({ kind: 'profile-editor', profileId: 'p1' }, 'Edit a')
  s.openView({ kind: 'profile-editor', profileId: 'p1' }, 'Edit a')
  s.openView({ kind: 'profile-editor', profileId: null }, 'New profile')
  expect(useSessionStore.getState().tabs).toHaveLength(2)
})

test('removeTab on a view tab drops only the tab, never a session', () => {
  const s = useSessionStore.getState()
  s.openTab({ id: 'sess1', profileId: 'p', title: 'web', status: 'connecting', reconnectNonce: 0 })
  s.openView({ kind: 'settings' }, 'Settings')
  const view = useSessionStore.getState().tabs.find((t) => t.kind === 'view')
  if (!view) throw new Error('expected a view tab')
  s.removeTab(view.id)
  expect(useSessionStore.getState().tabs).toHaveLength(1)
  expect(useSessionStore.getState().sessions.sess1).toBeDefined()
})
