import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ setSecret: vi.fn().mockResolvedValue('new-secret') }))
vi.mock('@/bindings', () => ({
  commands: { upsertKey: vi.fn().mockResolvedValue({ status: 'ok', data: null }) },
}))

import type { Connection, Host } from '@/bindings'
import { setSecret } from '@/lib/vault'
import { draftFrom, type HostData, jumpChoices, toConnection, validateDraft } from './drafts'

const ssh = (id: string, jumpHostId: string | null = null): Connection => ({
  id,
  label: null,
  address: null,
  kind: {
    type: 'ssh',
    port: 22,
    login: { kind: 'own', username: 'admin', authMethod: 'password', keyIds: [], secretId: 'old' },
    jumpHostId,
    tunnels: [],
    appearance: null,
  },
})

const host = (id: string, connections: Connection[]): Host => ({
  id,
  name: id,
  address: `${id}.lan`,
  folderId: null,
  color: null,
  order: 0,
  connections,
})

const file = (hosts: Host[]): HostData => ({ hosts, logins: [], keys: [] })

beforeEach(() => {
  vi.clearAllMocks()
})

test('ports outside 1 to 65535 are refused', () => {
  const d = draftFrom('ssh', null, null, file([]))
  for (const port of ['0', '70000', 'x', '']) {
    expect(validateDraft({ ...d, user: 'me', port }).port).toBe('Use a number from 1 to 65535.')
  }
  expect(validateDraft({ ...d, user: 'me', port: '2222' }).port).toBeUndefined()
})

test('a typed password is stored and replaces the secret id', async () => {
  const f = file([host('nas', [ssh('c')])])
  const d = draftFrom('ssh', f.hosts[0].connections[0], f.hosts[0], f)
  const kept = await toConnection(d, f)
  expect(kept.kind.type === 'ssh' && kept.kind.login).toMatchObject({ secretId: 'old' })
  expect(setSecret).not.toHaveBeenCalled()

  const changed = await toConnection({ ...d, password: 'hunter2' }, f)
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(changed.kind.type === 'ssh' && changed.kind.login).toMatchObject({
    secretId: 'new-secret',
  })
})

test('jump host choices leave out cycles', () => {
  const f = file([
    host('a', [ssh('a1', 'b1')]),
    host('b', [ssh('b1', 'c1')]),
    host('c', [ssh('c1')]),
  ])
  // c1 jumping through a1 or b1 would loop back to c1.
  expect(jumpChoices(f, 'c1').map((j) => j.id)).toEqual([])
  expect(jumpChoices(f, 'a1').map((j) => j.id)).toEqual(['b1', 'c1'])
  expect(jumpChoices(f, null).map((j) => j.label)).toEqual([
    'a · SSH · admin',
    'b · SSH · admin',
    'c · SSH · admin',
  ])
})
