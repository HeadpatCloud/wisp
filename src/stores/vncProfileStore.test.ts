import { beforeEach, expect, test, vi } from 'vitest'
import type { VncProfile } from '@/bindings'

const m = vi.hoisted(() => ({
  listVncProfiles: vi.fn(),
  upsertVncProfile: vi.fn(),
  deleteVncProfile: vi.fn(),
  deleteSecret: vi.fn(),
}))

vi.mock('@/bindings', () => ({ commands: m }))

import { useVncProfileStore } from './vncProfileStore'

const OK = { status: 'ok', data: null }
const desk: VncProfile = {
  id: 'vnc-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 5901,
  username: 'faye',
  secretId: 'vault-1',
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
}

beforeEach(() => {
  vi.resetAllMocks()
  useVncProfileStore.setState({ profiles: [], loaded: false })
  m.listVncProfiles.mockResolvedValue({ status: 'ok', data: [] })
  for (const command of [m.upsertVncProfile, m.deleteVncProfile, m.deleteSecret]) {
    command.mockResolvedValue(OK)
  }
})

test('load pulls the saved profiles', async () => {
  m.listVncProfiles.mockResolvedValue({ status: 'ok', data: [desk] })
  await useVncProfileStore.getState().load()
  expect(useVncProfileStore.getState()).toMatchObject({ profiles: [desk], loaded: true })
})

test('save upserts the profile, then reloads the list', async () => {
  const order: string[] = []
  m.upsertVncProfile.mockImplementation(async () => {
    order.push('upsert')
    return OK
  })
  m.listVncProfiles.mockImplementation(async () => {
    order.push('list')
    return { status: 'ok', data: [desk] }
  })
  await useVncProfileStore.getState().save(desk)
  expect(m.upsertVncProfile).toHaveBeenCalledWith(desk)
  expect(order).toEqual(['upsert', 'list'])
  expect(useVncProfileStore.getState().profiles).toEqual([desk])
})

test('a save the backend refuses rejects and keeps the list', async () => {
  useVncProfileStore.setState({ profiles: [desk] })
  m.upsertVncProfile.mockResolvedValue({
    status: 'error',
    error: { kind: 'io', message: 'disk full' },
  })
  await expect(useVncProfileStore.getState().save(desk)).rejects.toThrow('io: disk full')
  expect(m.listVncProfiles).not.toHaveBeenCalled()
  expect(useVncProfileStore.getState().profiles).toEqual([desk])
})

test('remove deletes the secret, then the profile, then reloads the list', async () => {
  useVncProfileStore.setState({ profiles: [desk], loaded: true })
  const order: string[] = []
  m.deleteSecret.mockImplementation(async (id: string) => {
    order.push(`secret ${id}`)
    return OK
  })
  m.deleteVncProfile.mockImplementation(async (id: string) => {
    order.push(`profile ${id}`)
    return OK
  })
  m.listVncProfiles.mockImplementation(async () => {
    order.push('list')
    return { status: 'ok', data: [] }
  })
  await useVncProfileStore.getState().remove('vnc-1')
  expect(order).toEqual(['secret vault-1', 'profile vnc-1', 'list'])
  expect(useVncProfileStore.getState().profiles).toEqual([])
})

test.each([
  ['is already gone', () => ({ status: 'error', error: { kind: 'notFound', message: 'secret' } })],
  [
    'cannot be reached',
    () => {
      throw new Error('ipc')
    },
  ],
])('a secret that %s does not keep the profile from being removed', async (_how, outcome) => {
  useVncProfileStore.setState({ profiles: [desk], loaded: true })
  m.deleteSecret.mockImplementation(async () => outcome())
  await useVncProfileStore.getState().remove('vnc-1')
  expect(m.deleteSecret).toHaveBeenCalledWith('vault-1')
  expect(m.deleteVncProfile).toHaveBeenCalledWith('vnc-1')
  expect(useVncProfileStore.getState().profiles).toEqual([])
})

test('removing a profile without a password touches no secret', async () => {
  useVncProfileStore.setState({ profiles: [{ ...desk, secretId: null }], loaded: true })
  await useVncProfileStore.getState().remove('vnc-1')
  expect(m.deleteSecret).not.toHaveBeenCalled()
  expect(m.deleteVncProfile).toHaveBeenCalledWith('vnc-1')
})

test('a profile the backend cannot delete rejects', async () => {
  useVncProfileStore.setState({ profiles: [desk], loaded: true })
  m.deleteVncProfile.mockResolvedValue({
    status: 'error',
    error: { kind: 'notFound', message: 'vnc profile vnc-1' },
  })
  await expect(useVncProfileStore.getState().remove('vnc-1')).rejects.toThrow(
    'notFound: vnc profile vnc-1',
  )
})
