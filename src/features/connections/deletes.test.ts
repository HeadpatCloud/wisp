import { expect, test } from 'vitest'
import type { Connection, Host } from '@/bindings'
import { connectionDeleteText, hostDeleteText } from './deletes'

const ssh = (id: string, jumpHostId: string | null = null, tunnels = 0): Connection => ({
  id,
  label: null,
  address: null,
  kind: {
    type: 'ssh',
    port: 22,
    login: { kind: 'own', username: 'ops', authMethod: 'agent', keyIds: [], secretId: null },
    jumpHostId,
    tunnels: Array.from({ length: tunnels }, (_, i) => ({
      id: `${id}-t${i}`,
      kind: 'local' as const,
      bindHost: '127.0.0.1',
      bindPort: 8000 + i,
      targetHost: 'x',
      targetPort: 80,
      autoStart: false,
    })),
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

test('the delete confirmation names jump host users', () => {
  const bastion = host('bastion', [ssh('b1', null, 1), ssh('b2')])
  const hosts = [bastion, host('build', [ssh('x1', 'b1')]), host('db', [ssh('y1', 'b2')])]
  expect(hostDeleteText(bastion, hosts, [])).toEqual({
    title: 'Delete bastion?',
    body:
      'Its 2 connections and 1 tunnel go with it. Saved logins and keys stay in Keys & passwords.' +
      ' build · SSH · ops and db · SSH · ops use it as their jump host and stop connecting until you pick another.',
  })
  expect(connectionDeleteText(bastion, bastion.connections[0], hosts, [])).toEqual({
    title: 'Delete SSH · ops?',
    body:
      'It goes when you save. Its tunnels go with it.' +
      ' build · SSH · ops uses it as its jump host and stops connecting until you pick another.',
  })
  expect(hostDeleteText(hosts[1], hosts, []).body).toBe(
    'Its 1 connection goes with it. Saved logins and keys stay in Keys & passwords.',
  )
})
