import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  getHosts: vi.fn(),
  upsertHost: vi.fn(),
  upsertFolder: vi.fn(),
  moveHosts: vi.fn(),
  moveFolders: vi.fn(),
  listShells: vi.fn(),
}))

vi.mock('@/bindings', () => ({ commands: m }))

import { useHostStore } from './hostStore'

const file = {
  version: 1,
  folders: [{ id: 'f', name: 'Work', parentId: null, order: 0 }],
  hosts: [
    {
      id: 'h',
      name: 'nas',
      address: 'nas.local',
      folderId: null,
      color: null,
      order: 0,
      connections: [],
    },
  ],
  logins: [],
  keys: [],
  tags: [],
}

beforeEach(() => {
  vi.clearAllMocks()
  m.getHosts.mockResolvedValue({ status: 'ok', data: file })
  for (const f of [m.upsertHost, m.upsertFolder, m.moveHosts, m.moveFolders]) {
    f.mockResolvedValue({ status: 'ok', data: null })
  }
  useHostStore.setState({ folders: [], hosts: [], logins: [], keys: [], tags: [], loaded: false })
})

test('loadShells keeps the shells this computer has', async () => {
  m.listShells.mockResolvedValue([{ name: 'PowerShell', program: 'pwsh.exe' }])
  await useHostStore.getState().loadShells()
  expect(useHostStore.getState().shells).toEqual([{ name: 'PowerShell', program: 'pwsh.exe' }])
})

test('load takes every list from the hosts file', async () => {
  await useHostStore.getState().load()
  const s = useHostStore.getState()
  expect(s.loaded).toBe(true)
  expect(s.hosts.map((h) => h.id)).toEqual(['h'])
  expect(s.folders.map((f) => f.name)).toEqual(['Work'])
})

test('each change is saved and then the file is read again', async () => {
  const s = useHostStore.getState()
  await s.saveHost(file.hosts[0] as never)
  await s.saveFolder(file.folders[0] as never)
  await s.moveHosts([{ id: 'h', folderId: 'f', order: 1 }])
  await s.moveFolders([{ id: 'f', parentId: null, order: 2 }])
  expect(m.upsertHost).toHaveBeenCalledWith(file.hosts[0], [])
  expect(m.upsertFolder).toHaveBeenCalledWith(file.folders[0])
  expect(m.moveHosts).toHaveBeenCalledWith([{ id: 'h', folderId: 'f', order: 1 }])
  expect(m.moveFolders).toHaveBeenCalledWith([{ id: 'f', parentId: null, order: 2 }])
  expect(m.getHosts).toHaveBeenCalledTimes(4)
})

test('a refused change is thrown and nothing is reloaded', async () => {
  m.moveFolders.mockResolvedValue({ status: 'error', error: { kind: 'invalid', message: 'x' } })
  await expect(useHostStore.getState().moveFolders([])).rejects.toThrow('x')
  expect(m.getHosts).not.toHaveBeenCalled()
})
