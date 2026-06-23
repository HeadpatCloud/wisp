import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  listGroups: vi.fn(),
  listProfiles: vi.fn(),
  upsertProfile: vi.fn(),
  deleteProfile: vi.fn(),
  deleteSecret: vi.fn(),
}))

vi.mock('@/bindings', () => ({
  commands: {
    listGroups: m.listGroups,
    listProfiles: m.listProfiles,
    upsertProfile: m.upsertProfile,
    deleteProfile: m.deleteProfile,
    deleteSecret: m.deleteSecret,
    upsertGroup: vi.fn(),
    deleteGroup: vi.fn(),
  },
}))

import { useProfileStore } from './profileStore'

const profile = { id: 'p1', name: 'web', port: 22, secretId: null } as never

beforeEach(() => {
  vi.clearAllMocks()
  useProfileStore.setState({ groups: [], profiles: [], loaded: false })
})

test('load pulls groups and profiles', async () => {
  m.listGroups.mockResolvedValue({ status: 'ok', data: [] })
  m.listProfiles.mockResolvedValue({ status: 'ok', data: [profile] })
  await useProfileStore.getState().load()
  expect(useProfileStore.getState().profiles).toHaveLength(1)
  expect(useProfileStore.getState().loaded).toBe(true)
})

test('saveProfile upserts then refreshes', async () => {
  m.upsertProfile.mockResolvedValue({ status: 'ok', data: null })
  m.listProfiles.mockResolvedValue({ status: 'ok', data: [profile] })
  await useProfileStore.getState().saveProfile(profile)
  expect(m.upsertProfile).toHaveBeenCalledWith(profile)
  expect(useProfileStore.getState().profiles).toHaveLength(1)
})

test('removeProfile deletes the linked secret first', async () => {
  useProfileStore.setState({
    profiles: [{ id: 'p1', secretId: 'sec1' } as never],
    groups: [],
    loaded: true,
  })
  m.deleteSecret.mockResolvedValue({ status: 'ok', data: null })
  m.deleteProfile.mockResolvedValue({ status: 'ok', data: null })
  m.listProfiles.mockResolvedValue({ status: 'ok', data: [] })
  await useProfileStore.getState().removeProfile('p1')
  expect(m.deleteSecret).toHaveBeenCalledWith('sec1')
  expect(m.deleteProfile).toHaveBeenCalledWith('p1')
})

test('a command error propagates', async () => {
  m.upsertProfile.mockResolvedValue({
    status: 'error',
    error: { kind: 'io', message: 'disk full' },
  })
  await expect(useProfileStore.getState().saveProfile(profile)).rejects.toThrow('io: disk full')
})
