import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ deleteSecret: vi.fn().mockResolvedValue(undefined) }))

import type { AdhocParams } from '@/lib/sftp'
import { deleteSecret } from '@/lib/vault'
import type { PanesTab, RdpTab, VncTab } from './sessionStore'
import { tabSecretIds, useSessionStore } from './sessionStore'

const st = () => useSessionStore.getState()
const tab = (i = 0) => st().tabs[i] as PanesTab

beforeEach(() => {
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  vi.mocked(deleteSecret).mockClear()
})

test('openSsh creates a one-pane tab and activates it', () => {
  st().openSsh('c1', 'web')
  expect(tab()).toMatchObject({ kind: 'panes', direction: 'horizontal' })
  expect(tab().paneIds).toHaveLength(1)
  expect(st().panes[tab().paneIds[0]]).toMatchObject({
    kind: 'ssh',
    connectionId: 'c1',
    title: 'web',
  })
  expect(st().activeTabId).toBe(tab().id)
})

test('opening with split adds the pane to the active panes tab', () => {
  st().openSsh('c1', 'web')
  st().openSftpProfile('f1', 'web files', { split: 'vertical' })
  expect(st().tabs).toHaveLength(1)
  expect(tab().paneIds).toHaveLength(2)
  expect(tab().direction).toBe('vertical')
  expect(st().panes[tab().activePaneId]).toMatchObject({ kind: 'sftp', connectionId: 'f1' })
})

test('a split opened from the Hosts tab goes to the panes tab used last', () => {
  st().openSsh('c1', 'web')
  st().openSsh('c2', 'db')
  const [web, db] = st().tabs
  st().setActiveTab(web.id)
  st().setActiveTab(null)
  st().openS3('s1', null, 'bucket', { split: 'horizontal' })
  expect(st().tabs).toHaveLength(2)
  expect(st().activeTabId).toBe(web.id)
  expect(tab(0).paneIds).toHaveLength(2)
  expect(tab(1).id).toBe(db.id)
})

test('a split from the Hosts tab finds its tab after restore, merge and duplicate', () => {
  st().openSsh('c1', 'web')
  st().openSsh('c2', 'db')
  const [web, db] = st().tabs as PanesTab[]
  // After a restart nothing was activated by hand yet.
  useSessionStore.setState({ lastPanesTabId: null })
  st().restoreTabs({ tabs: st().tabs, panes: st().panes, activeTabId: web.id })
  st().setActiveTab(null)
  st().openS3('s1', null, 'bucket', { split: 'horizontal' })
  expect(st().activeTabId).toBe(web.id)
  expect((st().tabs[0] as PanesTab).paneIds).toHaveLength(2)

  // Merged away: the tab it went into takes over.
  st().setActiveTab(db.id)
  st().mergeTab(db.id, web.id, 'horizontal')
  st().setActiveTab(null)
  st().openS3('s2', null, 'bucket', { split: 'horizontal' })
  expect(st().tabs).toHaveLength(1)
  expect((st().tabs[0] as PanesTab).paneIds).toHaveLength(4)

  // A duplicate is the tab used last.
  st().duplicateTab(web.id)
  const copy = st().tabs[1]
  st().setActiveTab(null)
  st().openS3('s3', null, 'bucket', { split: 'horizontal' })
  expect(st().activeTabId).toBe(copy.id)
})

test('a pane joining a broadcasting tab turns broadcast off', () => {
  st().openSsh('c1', 'web')
  const web = tab(0)
  st().toggleBroadcast(web.id)
  st().openSsh('c2', 'db', { split: 'horizontal' })
  expect(tab(0).broadcast).toBe(false)
  st().toggleBroadcast(web.id)
  st().openSsh('c3', 'prod')
  st().mergeTab(tab(1).id, web.id, 'horizontal')
  expect(tab(0).broadcast).toBe(false)
})

test('closing a terminal tab lets files on its session connect on their own', () => {
  st().openSsh('c1', 'web')
  const term = tab().paneIds[0]
  st().setSshId(term, 'ssh-1')
  st().openFilesBeside(tab().id, term)
  st().duplicateTab(tab().id)
  const copy = tab(1).paneIds[0]
  expect(st().panes[copy]).toMatchObject({ kind: 'sftp', viaSession: 'ssh-1' })
  st().removeTab(tab(0).id)
  expect(st().panes[copy]).toMatchObject({ viaSession: null, sshConnectionId: 'c1' })
})

test('files opened on a terminal follow its session', () => {
  st().openSsh('c1', 'web')
  const term = tab().paneIds[0]
  st().setSshId(term, 'ssh-1')
  st().openFilesBeside(tab().id, term)
  const files = tab().paneIds[1]
  expect(st().panes[files]).toMatchObject({
    kind: 'sftp',
    viaSession: 'ssh-1',
    sshConnectionId: 'c1',
  })
  st().setSshId(term, 'ssh-2')
  expect(st().panes[files]).toMatchObject({ viaSession: 'ssh-2' })
  st().closePane(tab().id, term)
  expect(st().panes[files]).toMatchObject({ viaSession: null, sshConnectionId: 'c1' })
})

test('splitPane copies the source pane after it and focuses the copy', () => {
  st().openLocalShell('pwsh', 'PowerShell', 'C:\\work', 'c-local')
  const first = tab().paneIds[0]
  st().splitPane(tab().id, first, 'vertical')
  expect(tab().paneIds[0]).toBe(first)
  expect(tab().activePaneId).toBe(tab().paneIds[1])
  expect(st().panes[tab().paneIds[1]]).toMatchObject({
    kind: 'local',
    program: 'pwsh',
    startFolder: 'C:\\work',
    connectionId: 'c-local',
  })
})

test('closePane removes one pane; closing the last removes the tab', () => {
  st().openSsh('c1', 'web')
  const first = tab().paneIds[0]
  st().splitPane(tab().id, first, 'horizontal')
  st().closePane(tab().id, tab().activePaneId)
  expect(tab().paneIds).toEqual([first])
  expect(tab().activePaneId).toBe(first)
  st().closePane(tab().id, first)
  expect(st().tabs).toHaveLength(0)
  expect(st().panes).toEqual({})
  expect(st().activeTabId).toBeNull()
})

test('mergeTab joins two panes tabs in one row', () => {
  st().openSsh('c1', 'web')
  st().openS3('s1', 'logs', 'bucket')
  const [into, from] = st().tabs as PanesTab[]
  st().mergeTab(from.id, into.id, 'horizontal')
  expect(st().tabs).toHaveLength(1)
  expect(tab().paneIds).toEqual([...into.paneIds, ...from.paneIds])
  expect(st().activeTabId).toBe(into.id)
  st().openLocalShell()
  const local = tab(1)
  st().mergeTab(local.id, into.id, 'vertical', true)
  expect(tab().paneIds).toEqual([...local.paneIds, ...into.paneIds, ...from.paneIds])
  expect(tab().direction).toBe('vertical')
})

test('move_tab_reorders_and_clamps', () => {
  st().openSsh('a', 'a')
  st().openSsh('b', 'b')
  st().openVnc({ once: { host: 'c', port: 5900, username: null, secretId: null } })
  const ids = () => st().tabs.map((t) => t.id)
  const [a, b, c] = ids()
  st().moveTab(a, 1)
  expect(ids()).toEqual([b, a, c])
  st().moveTab(c, -5)
  expect(ids()).toEqual([c, b, a])
  st().moveTab(a, 3)
  expect(ids()).toEqual([c, b, a])
  st().moveTabTo(c, 2)
  expect(ids()).toEqual([b, a, c])
})

test('reconnect bumps the nonce and sets connecting', () => {
  st().openSsh('c1', 'web')
  const id = tab().paneIds[0]
  st().setStatus(id, 'closed')
  st().reconnect(id)
  expect(st().panes[id]).toMatchObject({ status: 'connecting', reconnectNonce: 1 })
})

test('duplicateTab opens the active pane again in a new tab', () => {
  st().openFtpSaved('ftp-1', 'nas', {
    host: 'nas.local',
    port: 21,
    username: 'media',
    secure: false,
  })
  st().duplicateTab(tab().id)
  expect(st().tabs).toHaveLength(2)
  const [a, b] = [st().panes[tab(0).paneIds[0]], st().panes[tab(1).paneIds[0]]]
  expect(b).toMatchObject({ kind: 'ftp', connectionId: 'ftp-1' })
  expect(b.id).not.toBe(a.id)
  expect(st().activeTabId).toBe(tab(1).id)
})

test('setZoom stays within the range', () => {
  st().openLocalShell()
  const id = tab().paneIds[0]
  st().setZoom(id, 40)
  expect(st().panes[id].zoom).toBe(12)
  st().setZoom(id, -40)
  expect(st().panes[id].zoom).toBe(-6)
})

const adhoc: AdhocParams = {
  host: 'box',
  port: 22,
  username: 'root',
  secretId: 'pw-1',
  keys: [{ path: 'C:\\id', secretId: 'kp-1' }],
} as AdhocParams

test('ad_hoc_secrets_are_released_once', () => {
  st().openSshAdhoc(adhoc)
  const sshTab = tab().id
  st().splitPane(sshTab, tab().paneIds[0], 'horizontal')
  st().openFtp({
    host: 'ftp.box',
    port: 21,
    username: 'u',
    secretId: 'ftp-pw',
    secure: false,
    allowInvalidCert: false,
    ignoreHostname: false,
  })
  st().mergeTab(tab(1).id, sshTab, 'horizontal')
  expect(deleteSecret).not.toHaveBeenCalled()

  const [copyA, copyB, ftp] = tab().paneIds
  st().closePane(sshTab, copyA)
  expect(deleteSecret).not.toHaveBeenCalled()
  st().closePane(sshTab, ftp)
  expect(vi.mocked(deleteSecret).mock.calls).toEqual([['ftp-pw']])
  st().removeTab(sshTab)
  expect(vi.mocked(deleteSecret).mock.calls.flat().sort()).toEqual(['ftp-pw', 'kp-1', 'pw-1'])
  expect(st().panes[copyB]).toBeUndefined()
})

test('a saved ftp pane owns no secret', () => {
  st().openFtpSaved('ftp-1', 'nas', {
    host: 'nas.local',
    port: 21,
    username: 'media',
    secure: false,
  })
  const pane = st().panes[tab().paneIds[0]]
  expect(tabSecretIds(tab(), { [pane.id]: { ...pane, secretId: 'saved' } as typeof pane })).toEqual(
    [],
  )
})

const vncTab: VncTab = {
  id: 'v1',
  kind: 'vnc',
  title: 'mac:5900',
  connectionId: null,
  once: { host: 'mac', port: 5900, username: null, secretId: 'vault-1' },
}

const rdpTab: RdpTab = {
  id: 'r1',
  kind: 'rdp',
  title: 'pc:3389',
  connectionId: null,
  once: {
    host: 'pc',
    port: 3389,
    username: 'alice',
    domain: null,
    secretId: 'vault-1',
    keyboardLayout: null,
  },
}

test('connect-once remote desktop tabs own their secret, saved ones do not', () => {
  expect(tabSecretIds(vncTab)).toEqual(['vault-1'])
  expect(tabSecretIds({ ...vncTab, connectionId: 'vnc-1', once: null })).toEqual([])
  expect(tabSecretIds(rdpTab)).toEqual(['vault-1'])
  expect(tabSecretIds({ ...rdpTab, connectionId: 'rdp-1', once: null })).toEqual([])
})
