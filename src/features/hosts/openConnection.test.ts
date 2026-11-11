import { beforeEach, expect, test, vi } from 'vitest'
import type { Connection, Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useSessionStore } from '@/stores/sessionStore'
import { openConnection, openFiles } from './openConnection'

const own = {
  kind: 'own',
  username: 'admin',
  authMethod: 'password',
  keyIds: [],
  secretId: null,
} as const

function host(connections: Connection[]): Host {
  return {
    id: 'h',
    name: 'nas',
    address: 'nas.local',
    folderId: null,
    color: null,
    order: 0,
    connections,
  }
}

const c = (id: string, kind: Connection['kind']): Connection => ({
  id,
  label: null,
  address: null,
  kind,
})

const spies = {
  openSsh: vi.fn(),
  openSftp: vi.fn(),
  openSftpProfile: vi.fn(),
  openS3: vi.fn(),
  openVnc: vi.fn(),
  openRdp: vi.fn(),
  openLocalShell: vi.fn(),
}

beforeEach(() => {
  for (const s of Object.values(spies)) s.mockReset()
  useSessionStore.setState(spies as never)
  useHostStore.setState({ shells: [] })
})

test('ssh opens a terminal tab on the connection id', () => {
  const ssh = c('s', {
    type: 'ssh',
    port: 22,
    login: { ...own, keyIds: [] },
    jumpHostId: null,
    tunnels: [],
    appearance: null,
  })
  openConnection(host([ssh]), ssh)
  expect(spies.openSsh).toHaveBeenCalledWith('s', 'nas', { split: undefined })
})

test('files, buckets and desktops open with the connection id', () => {
  const sftp = c('f', { type: 'sftp', own: null, startFolder: null })
  const s3 = c('b', {
    type: 's3',
    port: null,
    region: 'eu',
    useTls: true,
    pathStyle: false,
    accessKeyId: 'AK',
    secretId: null,
    bucket: 'backups',
  })
  const vnc = {
    ...c('v', { type: 'vnc', port: 5901, username: 'faye', secretId: null }),
    address: '10.0.0.5',
  }
  const rdp = c('r', {
    type: 'rdp',
    port: 3389,
    login: { ...own, keyIds: [] },
    domain: 'corp',
    keyboardLayout: 0x407,
  })
  const h = host([sftp, s3, vnc, rdp])
  openConnection(h, sftp)
  openConnection(h, s3)
  openConnection(h, vnc)
  openConnection(h, rdp)
  expect(spies.openSftpProfile).toHaveBeenCalledWith('f', 'nas', { split: undefined })
  expect(spies.openS3).toHaveBeenCalledWith('b', 'backups', 'nas', { split: undefined })
  expect(spies.openVnc).toHaveBeenCalledWith({ connectionId: 'v', title: 'nas' })
  expect(spies.openRdp).toHaveBeenCalledWith({ connectionId: 'r', title: 'nas' })
})

test('a local shell tab is titled with the shell name', () => {
  useHostStore.setState({ shells: [{ name: 'PowerShell', program: 'pwsh.exe' }] })
  const local = c('l', { type: 'local', program: 'pwsh.exe', startFolder: null })
  openConnection(host([local]), local)
  expect(spies.openLocalShell).toHaveBeenCalledWith('pwsh.exe', 'PowerShell', null, 'l', {
    split: undefined,
  })
})

test('a local shell opens its program', () => {
  const local = c('l', { type: 'local', program: 'pwsh.exe', startFolder: null })
  openConnection(host([local]), local)
  expect(spies.openLocalShell).toHaveBeenCalledWith('pwsh.exe', 'pwsh.exe', null, 'l', {
    split: undefined,
  })
})

test('openFiles opens SFTP over an SSH connection', () => {
  openFiles(host([]), 's')
  expect(spies.openSftp).toHaveBeenCalledWith('s', 'nas', { split: undefined })
})
