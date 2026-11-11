import { describe, expect, test } from 'vitest'
import type { Connection, Host, Login, SavedLogin, Tag } from '@/bindings'
import {
  connectionLabel,
  effectiveAddress,
  findConnection,
  folderPath,
  hostMatches,
  s3View,
  sshView,
  typeGroup,
  viaLine,
} from './hosts'

const own = (username: string): Login => ({
  kind: 'own',
  username,
  authMethod: 'password',
  keyIds: [],
  secretId: null,
})

function conn(id: string, kind: Connection['kind'], label: string | null = null): Connection {
  return { id, label, address: null, kind }
}

function host(
  id: string,
  name: string,
  connections: Connection[],
  extra: Partial<Host> = {},
): Host {
  return {
    id,
    name,
    address: `${name}.lan`,
    folderId: null,
    tagIds: [],
    color: null,
    order: 0,
    connections,
    ...extra,
  }
}

const ssh = (id: string, user: string, label: string | null = null) =>
  conn(
    id,
    { type: 'ssh', port: 22, login: own(user), jumpHostId: null, tunnels: [], appearance: null },
    label,
  )

const saved: SavedLogin[] = [
  { id: 'corp', name: 'corp-dev', username: 'CORP\\dev', authMethod: 'password', secretId: null },
]

describe('connectionLabel', () => {
  const nas = host('h', 'nas', [])
  test.each([
    ['SSH with its user', ssh('a', 'admin'), 'SSH · admin'],
    ['SSH with a label', ssh('a', 'admin', 'root'), 'SSH · root'],
    ['Files', conn('f', { type: 'sftp', own: null, startFolder: null }), 'Files'],
    [
      'a bucket',
      conn('b', {
        type: 's3',
        port: null,
        region: 'eu',
        useTls: true,
        pathStyle: false,
        accessKeyId: 'AK',
        secretId: null,
        bucket: 'b',
      }),
      'Bucket b',
    ],
    [
      'RDP with a saved login',
      conn('r', {
        type: 'rdp',
        port: 3389,
        login: { kind: 'saved', loginId: 'corp' },
        domain: null,
        keyboardLayout: null,
      }),
      'RDP · CORP\\dev',
    ],
    [
      'a local shell with a start folder',
      conn('l', { type: 'local', program: 'C:\\Git\\bin\\bash.exe', startFolder: 'D:\\git' }),
      'Git Bash · D:\\git',
    ],
  ])('%s', (_, c, expected) => {
    const shells = [{ name: 'Git Bash', program: 'C:\\Git\\bin\\bash.exe' }]
    expect(connectionLabel(nas, c, saved, shells)).toBe(expected)
  })
})

test('a connection address overrides the host address', () => {
  const c = { ...ssh('a', 'u'), address: '10.9.9.9' }
  expect(effectiveAddress(host('h', 'nas', [c]), c)).toBe('10.9.9.9')
  expect(effectiveAddress(host('h', 'nas', []), ssh('b', 'u'))).toBe('nas.lan')
})

test('findConnection returns its host, or null for an unknown id', () => {
  const nas = host('h', 'nas', [ssh('a', 'u')])
  expect(findConnection([nas], 'a')?.host.id).toBe('h')
  expect(findConnection([nas], 'nope')).toBeNull()
})

test('folderPath joins nested folder names', () => {
  const folders = [
    { id: 'w', name: 'Work', parentId: null, order: 0 },
    { id: 's', name: 'Servers', parentId: 'w', order: 0 },
  ]
  expect(folderPath('s', folders)).toBe('Work / Servers')
  expect(folderPath(null, folders)).toBe('')
})

test('hostMatches finds a host by name, address or tag name', () => {
  const tags: Tag[] = [
    {
      id: 't',
      name: 'prod',
      color: 'red',
      showColor: true,
      askBeforeClose: true,
      askBeforePaste: true,
      terminalColors: null,
    },
  ]
  const db = host('h', 'db', [], { tagIds: ['t'], address: '10.0.9.3' })
  expect(hostMatches(db, 'DB', tags)).toBe(true)
  expect(hostMatches(db, '10.0.9', tags)).toBe(true)
  expect(hostMatches(db, 'Prod', tags)).toBe(true)
  expect(hostMatches(db, 'web', tags)).toBe(false)
})

test('typeGroup sorts connection types into terminal, files and desktop', () => {
  expect([typeGroup('ssh'), typeGroup('local')]).toEqual(['terminal', 'terminal'])
  expect([typeGroup('sftp'), typeGroup('ftp'), typeGroup('s3')]).toEqual([
    'files',
    'files',
    'files',
  ])
  expect([typeGroup('rdp'), typeGroup('vnc')]).toEqual(['desktop', 'desktop'])
})

test('viaLine names the jump host of the main SSH connection', () => {
  const bastion = host('b', 'bastion', [ssh('b-ssh', 'u')])
  const behind = ssh('t-ssh', 'u')
  if (behind.kind.type === 'ssh') behind.kind.jumpHostId = 'b-ssh'
  const target = host('t', 'build-01', [behind])
  expect(viaLine(target, [bastion, target])).toBe('via bastion')
  expect(viaLine(bastion, [bastion, target])).toBeNull()
})

test('sshView resolves user, address, port, look and tunnels of an SSH connection', () => {
  const c = ssh('s', 'admin')
  if (c.kind.type === 'ssh') {
    c.kind.port = 2222
    c.kind.appearance = { colors: 'dracula', fontFamily: null, fontSize: 15 }
  }
  const nas = host('h', 'nas', [
    c,
    conn('v', { type: 'vnc', port: 5900, username: null, secretId: null }),
  ])
  expect(sshView([nas], [], 's')).toEqual({
    user: 'admin',
    host: 'nas.lan',
    port: 2222,
    appearance: { colors: 'dracula', fontFamily: null, fontSize: 15 },
    tunnels: [],
  })
  expect(sshView([nas], [], 'v')).toBeNull()
  expect(sshView([nas], [], undefined)).toBeNull()
})

test('s3View gives the endpoint settings of a bucket connection', () => {
  const b = conn('b', {
    type: 's3',
    port: 9000,
    region: 'eu',
    useTls: false,
    pathStyle: true,
    accessKeyId: 'AK',
    secretId: null,
    bucket: null,
  })
  const minio = host('h', 'minio', [b], { address: 'minio.lan' })
  expect(s3View([minio], 'b')).toEqual({
    endpoint: 'minio.lan',
    port: 9000,
    useTls: false,
    pathStyle: true,
  })
  expect(s3View([minio], 'x')).toBeNull()
})
