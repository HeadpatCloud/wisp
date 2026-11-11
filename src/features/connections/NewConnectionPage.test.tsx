import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ setSecret: vi.fn().mockResolvedValue('secret-1') }))
vi.mock('../hosts/openConnection', () => ({ openConnection: vi.fn() }))

import type { Connection, Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { openConnection } from '../hosts/openConnection'
import { NewConnectionPage } from './NewConnectionPage'

const sshConn: Connection = {
  id: 'nas-ssh',
  label: null,
  address: null,
  kind: {
    type: 'ssh',
    port: 22,
    login: { kind: 'own', username: 'admin', authMethod: 'agent', keyIds: [], secretId: null },
    jumpHostId: null,
    tunnels: [],
    appearance: null,
  },
}
const nas: Host = {
  id: 'nas',
  name: 'nas',
  address: 'nas.local',
  folderId: 'homelab',
  color: null,
  order: 0,
  connections: [sshConn],
}
const saveHost = vi.fn().mockResolvedValue([])

beforeEach(() => {
  vi.clearAllMocks()
  useHostStore.setState({
    folders: [{ id: 'homelab', name: 'Homelab', parentId: null, order: 0 }],
    hosts: [nas],
    logins: [],
    keys: [],
    tags: [],
    shells: [],
    loaded: true,
    saveHost,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
})

test('saving to a new host writes the host with its connection', async () => {
  const user = userEvent.setup()
  render(
    <NewConnectionPage
      prefill={{ type: 'ssh', address: 'box.lan', user: 'me' }}
      onClose={vi.fn()}
    />,
  )
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveHost).toHaveBeenCalledTimes(1)
  const host = saveHost.mock.calls[0][0] as Host
  expect(host).toMatchObject({ name: 'box.lan', address: 'box.lan', folderId: null })
  expect(host.connections).toHaveLength(1)
  expect(host.connections[0].kind).toMatchObject({
    type: 'ssh',
    port: 22,
    login: { kind: 'own', username: 'me' },
  })
})

test('a matching address adds to that host by default', async () => {
  const user = userEvent.setup()
  render(<NewConnectionPage prefill={{ type: 'vnc', address: 'NAS.local' }} onClose={vi.fn()} />)
  expect(screen.getByText('nas is already a host in Homelab, with SSH.')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Add to nas' })).toHaveAttribute('aria-pressed', 'true')
  await user.click(screen.getByRole('button', { name: 'Save' }))
  const host = saveHost.mock.calls[0][0] as Host
  expect(host.id).toBe('nas')
  expect(host.connections.map((c) => c.kind.type)).toEqual(['ssh', 'vnc'])
})

test('save and connect opens the new connection', async () => {
  const user = userEvent.setup()
  render(<NewConnectionPage prefill={{ type: 'vnc', address: 'desk.lan' }} onClose={vi.fn()} />)
  await user.click(screen.getByRole('button', { name: 'Save and connect' }))
  const host = saveHost.mock.calls[0][0] as Host
  expect(openConnection).toHaveBeenCalledWith(host, host.connections[0])
})
