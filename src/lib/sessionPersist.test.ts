import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ deleteSecret: vi.fn().mockResolvedValue(undefined) }))

import { deleteSecret } from '@/lib/vault'
import { type PanesTab, useSessionStore } from '@/stores/sessionStore'
import { clearSnapshot, dropMissing, loadSnapshot, saveSnapshot } from './sessionPersist'

const st = () => useSessionStore.getState()
const stored = (snap: object) => localStorage.setItem('wisp.session', JSON.stringify(snap))

beforeEach(() => {
  vi.mocked(deleteSecret).mockClear()
  localStorage.clear()
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
})

// The whole restart path: live store -> disk -> fresh store.
test('round-trips the store and arms every pane for reconnect', () => {
  st().openSsh('c1', 'web')
  const t = st().tabs[0] as PanesTab
  const id = t.paneIds[0]
  st().setStatus(id, 'connected')
  st().setSshId(id, 'live-ssh-id')
  st().toggleBroadcast(t.id)
  st().openFtp({
    host: 'h',
    port: 21,
    username: 'u',
    secretId: 'vault-1',
    secure: true,
    allowInvalidCert: false,
    ignoreHostname: false,
  })
  st().setActiveTab(t.id)
  saveSnapshot(st())

  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  const snap = loadSnapshot()
  if (!snap) throw new Error('nothing persisted')
  st().restoreTabs(snap)

  expect(st().tabs).toHaveLength(2)
  expect(st().activeTabId).toBe(t.id)
  // Live fields must not survive, or the pane would render as already connected.
  expect(st().panes[id]).toMatchObject({ status: 'connecting', sshId: null, connectionId: 'c1' })
  // Broadcast is never restored armed.
  expect((st().tabs[0] as PanesTab).broadcast).toBe(false)
  // Ids only - a snapshot must never be able to leak a usable credential.
  expect(localStorage.getItem('wisp.tabs')).toContain('vault-1')
})

const adhoc = {
  host: 'box',
  port: 22,
  username: 'root',
  authMethod: 'password',
  keys: [],
  secretId: 'vault-3',
}

test('old_snapshots_convert_to_panes', () => {
  // 0.5.0: profile ids, a view tab, VNC/RDP tabs from before they had a login.
  stored({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1', 's2'],
        direction: 'vertical',
        activePaneId: 's2',
      },
      { id: 'v1', kind: 'view', view: { kind: 'settings' }, title: 'Settings' },
      { id: 't2', kind: 'local', title: 'Local shell', program: 'pwsh', zoom: 2 },
      { id: 't3', kind: 'sftp', title: 'web', profileId: 'p1', sftpProfileId: null, adhoc: null },
      { id: 't4', kind: 's3', title: 'logs', profileId: 'p-s3', bucket: 'logs' },
      { id: 't5', kind: 'vnc', title: 'mac:5900', host: 'mac', port: 5900, secretId: null },
      { id: 't6', kind: 'rdp', title: 'pc:3389', host: 'pc', port: 3389 },
    ],
    sessions: {
      s1: {
        id: 's1',
        profileId: 'p1',
        title: 'web',
        status: 'connected',
        reconnectNonce: 2,
        sshId: 'x',
      },
      s2: {
        id: 's2',
        profileId: 'p1',
        title: 'web',
        status: 'connected',
        reconnectNonce: 0,
        zoom: 1,
      },
    },
    activeTabId: 't3',
  })
  const old = loadSnapshot()
  if (!old) throw new Error('nothing loaded')
  expect(old.tabs.map((t) => [t.id, t.kind])).toEqual([
    ['t1', 'panes'],
    ['t2', 'panes'],
    ['t3', 'panes'],
    ['t4', 'panes'],
    ['t5', 'vnc'],
    ['t6', 'rdp'],
  ])
  expect(old.activeTabId).toBe('t3')
  const [ssh, local, sftp, s3] = old.tabs as PanesTab[]
  expect(ssh).toMatchObject({ paneIds: ['s1', 's2'], direction: 'vertical', activePaneId: 's2' })
  expect(old.panes.s1).toMatchObject({ kind: 'ssh', connectionId: 'p1', sshId: null })
  expect(old.panes.s2).toMatchObject({ kind: 'ssh', zoom: 1 })
  expect(old.panes[local.paneIds[0]]).toMatchObject({
    kind: 'local',
    program: 'pwsh',
    startFolder: null,
    connectionId: null,
    zoom: 2,
  })
  expect(old.panes[sftp.paneIds[0]]).toMatchObject({
    kind: 'sftp',
    connectionId: null,
    sshConnectionId: 'p1',
  })
  expect(old.panes[s3.paneIds[0]]).toMatchObject({
    kind: 's3',
    connectionId: 'p-s3',
    bucket: 'logs',
  })
  expect(old.tabs[4]).toMatchObject({
    connectionId: null,
    once: { host: 'mac', port: 5900, username: null, secretId: null },
  })
  expect(old.tabs[5]).toMatchObject({
    connectionId: null,
    once: { username: '', domain: null, secretId: null, keyboardLayout: null },
  })

  // Checkpoint 2: connect-once SSH panes, saved FTP and Files connections, start folders.
  stored({
    tabs: [
      { id: 'a', kind: 'session', sessionIds: ['s9'], direction: 'horizontal', activePaneId: 's9' },
      { id: 'b', kind: 'sftp', title: 'nas', profileId: null, sftpProfileId: 'f1', adhoc: null },
      {
        id: 'c',
        kind: 'ftp',
        title: 'nas',
        connectionId: 'ftp-1',
        host: 'nas',
        port: 21,
        username: 'm',
        secretId: null,
        secure: false,
        allowInvalidCert: false,
        ignoreHostname: false,
      },
      { id: 'd', kind: 'local', title: 'Work', program: null, startFolder: 'C:\\work' },
    ],
    sessions: { s9: { id: 's9', profileId: '', title: 'root@box', adhoc } },
    activeTabId: 'gone',
  })
  const cp2 = loadSnapshot()
  if (!cp2) throw new Error('nothing loaded')
  expect(cp2.activeTabId).toBeNull()
  const [once, files, ftp, work] = cp2.tabs as PanesTab[]
  expect(cp2.panes[once.paneIds[0]]).toMatchObject({ kind: 'ssh', connectionId: null, adhoc })
  expect(cp2.panes[files.paneIds[0]]).toMatchObject({
    kind: 'sftp',
    connectionId: 'f1',
    sshConnectionId: null,
  })
  expect(cp2.panes[ftp.paneIds[0]]).toMatchObject({ kind: 'ftp', connectionId: 'ftp-1' })
  expect(cp2.panes[work.paneIds[0]]).toMatchObject({ kind: 'local', startFolder: 'C:\\work' })
})

test('a_restored_tab_whose_connection_is_gone_is_dropped', () => {
  st().openSsh('kept', 'web')
  const t = st().tabs[0] as PanesTab
  st().openSsh('deleted', 'old', { split: 'horizontal' })
  st().openSshAdhoc(adhoc as never)
  st().openS3('deleted', null, 'bucket')
  st().openVnc({ connectionId: 'deleted', title: 'mac' })
  st().openVnc({ once: { host: 'mac', port: 5900, username: null, secretId: null } })
  saveSnapshot(st())

  const snap = loadSnapshot()
  if (!snap) throw new Error('nothing persisted')
  const kept = dropMissing(snap, new Set(['kept']))
  expect(kept.tabs.map((x) => x.kind)).toEqual(['panes', 'panes', 'vnc'])
  const first = kept.tabs[0] as PanesTab
  expect(first.id).toBe(t.id)
  expect(first.paneIds).toEqual([t.paneIds[0]])
  expect(first.activePaneId).toBe(t.paneIds[0])
  expect(kept.panes[(kept.tabs[1] as PanesTab).paneIds[0]]).toMatchObject({ connectionId: null })
  expect(kept.activeTabId).toBe(kept.tabs[2].id)
})

test('returns null when nothing restorable was stored', () => {
  stored({
    tabs: [{ id: 'v', kind: 'view', view: { kind: 'welcome' }, title: 'Welcome' }],
    sessions: {},
    activeTabId: 'v',
  })
  expect(loadSnapshot()).toBeNull()
})

test('the_v2_snapshot_never_touches_the_0_5_0_key', () => {
  st().openSsh('c1', 'web')
  saveSnapshot(st())
  expect(localStorage.getItem('wisp.session')).toBeNull()

  // A 0.5.0 snapshot is converted once, then moved to the new key so 0.5.0 never reads 0.6.0's.
  clearSnapshot()
  stored({
    tabs: [{ id: 't1', kind: 'local', title: 'Local shell', program: null }],
    sessions: {},
    activeTabId: 't1',
  })
  expect(loadSnapshot()?.tabs.map((t) => t.id)).toEqual(['t1'])
  expect(localStorage.getItem('wisp.session')).toBeNull()
  expect(loadSnapshot()?.tabs.map((t) => t.id)).toEqual(['t1'])
})

test('a 0.5.0 snapshot written after going back wins, and frees what the newer one held', () => {
  st().openSshAdhoc({ ...adhoc, secretId: 'only-in-v2' } as never)
  saveSnapshot(st())
  stored({
    tabs: [{ id: 'old', kind: 'local', title: 'Local shell', program: null }],
    sessions: {},
    activeTabId: 'old',
  })
  expect(loadSnapshot()?.tabs.map((t) => t.id)).toEqual(['old'])
  expect(deleteSecret).toHaveBeenCalledWith('only-in-v2')
})

test('a malformed snapshot restores what it can', () => {
  localStorage.setItem(
    'wisp.tabs',
    JSON.stringify({
      v: 2,
      tabs: [
        null,
        { id: 'a', kind: 'panes' },
        { id: 'b', kind: 'panes', paneIds: ['gone', 'p1'], activePaneId: 'gone' },
        { id: 'c', kind: 'nonsense' },
        { id: 'd', kind: 'vnc', title: 'mac', connectionId: 'v1', once: null },
      ],
      panes: { p1: { id: 'p1', kind: 'local', title: 'x' }, p2: null, p3: { kind: 'ssh' } },
      activeTabId: 'a',
    }),
  )
  const snap = loadSnapshot()
  expect(snap?.tabs.map((t) => t.id)).toEqual(['b', 'd'])
  expect(snap?.tabs[0]).toMatchObject({
    paneIds: ['p1'],
    activePaneId: 'p1',
    direction: 'horizontal',
  })
  expect(Object.keys(snap?.panes ?? {})).toEqual(['p1'])
  expect(snap?.activeTabId).toBeNull()
})
