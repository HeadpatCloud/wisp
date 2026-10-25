import { beforeEach, expect, test, vi } from 'vitest'
import type { RdpProfile } from '@/bindings'

const m = vi.hoisted(() => ({
  listRdpProfiles: vi.fn(),
  upsertRdpProfile: vi.fn(),
  deleteRdpProfile: vi.fn(),
  deleteSecret: vi.fn(),
}))

vi.mock('@/bindings', () => ({ commands: m }))

import { useRdpProfileStore } from './rdpProfileStore'

const OK = { status: 'ok', data: null }
const desk: RdpProfile = {
  id: 'rdp-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 3389,
  username: 'faye',
  domain: null,
  secretId: 'vault-1',
  keyboardLayout: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
}

beforeEach(() => {
  vi.resetAllMocks()
  useRdpProfileStore.setState({ profiles: [], loaded: false })
  m.listRdpProfiles.mockResolvedValue({ status: 'ok', data: [] })
  for (const command of [m.upsertRdpProfile, m.deleteRdpProfile, m.deleteSecret]) {
    command.mockResolvedValue(OK)
  }
})

test('load pulls the saved profiles', async () => {
  m.listRdpProfiles.mockResolvedValue({ status: 'ok', data: [desk] })
  await useRdpProfileStore.getState().load()
  expect(useRdpProfileStore.getState()).toMatchObject({ profiles: [desk], loaded: true })
})

test('save upserts the profile and holds it without asking for the list', async () => {
  m.listRdpProfiles.mockResolvedValue({
    status: 'error',
    error: { kind: 'io', message: 'disk full' },
  })
  const other = { ...desk, id: 'rdp-2', name: 'other' }
  useRdpProfileStore.setState({ profiles: [other] })
  await useRdpProfileStore.getState().save(desk)
  expect(m.upsertRdpProfile).toHaveBeenCalledWith(desk)
  expect(m.listRdpProfiles).not.toHaveBeenCalled()
  expect(useRdpProfileStore.getState().profiles).toEqual([other, desk])
})

test('save puts a changed profile in the place of the one with its id', async () => {
  const other = { ...desk, id: 'rdp-2', name: 'other' }
  const renamed = { ...desk, name: 'renamed', secretId: 'vault-2' }
  useRdpProfileStore.setState({ profiles: [desk, other] })
  await useRdpProfileStore.getState().save(renamed)
  expect(m.listRdpProfiles).not.toHaveBeenCalled()
  expect(useRdpProfileStore.getState().profiles).toEqual([renamed, other])
})

test('a save the backend refuses rejects and keeps the list', async () => {
  useRdpProfileStore.setState({ profiles: [desk] })
  m.upsertRdpProfile.mockResolvedValue({
    status: 'error',
    error: { kind: 'io', message: 'disk full' },
  })
  await expect(useRdpProfileStore.getState().save(desk)).rejects.toThrow('io: disk full')
  expect(m.listRdpProfiles).not.toHaveBeenCalled()
  expect(useRdpProfileStore.getState().profiles).toEqual([desk])
})

test('remove deletes the profile, then its secret, then reloads the list', async () => {
  useRdpProfileStore.setState({ profiles: [desk], loaded: true })
  const order: string[] = []
  m.deleteSecret.mockImplementation(async (id: string) => {
    order.push(`secret ${id}`)
    return OK
  })
  m.deleteRdpProfile.mockImplementation(async (id: string) => {
    order.push(`profile ${id}`)
    return OK
  })
  m.listRdpProfiles.mockImplementation(async () => {
    order.push('list')
    return { status: 'ok', data: [] }
  })
  await useRdpProfileStore.getState().remove('rdp-1')
  expect(order).toEqual(['profile rdp-1', 'secret vault-1', 'list'])
  expect(useRdpProfileStore.getState().profiles).toEqual([])
})

test.each([
  ['is already gone', () => ({ status: 'error', error: { kind: 'notFound', message: 'secret' } })],
  [
    'cannot be reached',
    () => {
      throw new Error('ipc')
    },
  ],
])('a secret that %s does not fail the removal', async (_how, outcome) => {
  useRdpProfileStore.setState({ profiles: [desk], loaded: true })
  m.deleteSecret.mockImplementation(async () => outcome())
  await useRdpProfileStore.getState().remove('rdp-1')
  expect(m.deleteSecret).toHaveBeenCalledWith('vault-1')
  expect(m.deleteRdpProfile).toHaveBeenCalledWith('rdp-1')
  expect(m.listRdpProfiles).toHaveBeenCalledTimes(1)
  expect(useRdpProfileStore.getState().profiles).toEqual([])
})

test('removing a profile without a password touches no secret', async () => {
  useRdpProfileStore.setState({ profiles: [{ ...desk, secretId: null }], loaded: true })
  await useRdpProfileStore.getState().remove('rdp-1')
  expect(m.deleteSecret).not.toHaveBeenCalled()
  expect(m.deleteRdpProfile).toHaveBeenCalledWith('rdp-1')
})

test.each([
  [
    'the backend cannot delete',
    'io: disk full',
    () => ({ status: 'error', error: { kind: 'io', message: 'disk full' } }),
  ],
  [
    'cannot be reached',
    'ipc',
    () => {
      throw new Error('ipc')
    },
  ],
])('a profile that %s rejects and keeps its secret', async (_how, message, outcome) => {
  useRdpProfileStore.setState({ profiles: [desk], loaded: true })
  m.deleteRdpProfile.mockImplementation(async () => outcome())
  await expect(useRdpProfileStore.getState().remove('rdp-1')).rejects.toThrow(message)
  expect(m.deleteSecret).not.toHaveBeenCalled()
  expect(m.listRdpProfiles).not.toHaveBeenCalled()
  expect(useRdpProfileStore.getState().profiles).toEqual([desk])
})
