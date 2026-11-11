import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const update = vi.hoisted(() => vi.fn())
vi.mock('./openConnection', () => ({ openConnection: vi.fn(), openFiles: vi.fn() }))

import type { Connection, Host, Login } from '@/bindings'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Notices } from '@/components/Notices'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { HostsPage } from './HostsPage'
import { openConnection } from './openConnection'

const own = (username: string): Login => ({
  kind: 'own',
  username,
  authMethod: 'agent',
  keyIds: [],
  secretId: null,
})

function ssh(id: string, user: string, jump: string | null = null): Connection {
  return {
    id,
    label: null,
    address: null,
    kind: {
      type: 'ssh',
      port: 22,
      login: own(user),
      jumpHostId: jump,
      tunnels: [],
      appearance: null,
    },
  }
}

const sftp = (id: string): Connection => ({
  id,
  label: null,
  address: null,
  kind: { type: 'sftp', own: null, startFolder: null },
})

function host(
  id: string,
  name: string,
  address: string,
  folderId: string | null,
  connections: Connection[],
  extra: Partial<Host> = {},
): Host {
  return { id, name, address, folderId, color: null, order: 0, connections, ...extra }
}

const pc = host('this-computer', 'This computer', '', null, [
  {
    id: 'pc-1',
    label: null,
    address: null,
    kind: { type: 'local', program: 'pwsh.exe', startFolder: null },
  },
])
const nas = host('nas', 'nas.local', '192.168.1.20', 'homelab', [
  ssh('nas-ssh', 'admin'),
  sftp('nas-files'),
])
const win = host('win', 'win-dev', 'win-dev.corp', 'work', [ssh('win-ssh', 'dev')])
const bastion = host('bastion', 'bastion', 'bastion.corp', null, [ssh('b-ssh', 'ops')])
const build = host(
  'build',
  'build-01',
  '10.0.4.12',
  'servers',
  [ssh('build-ssh', 'deploy', 'b-ssh')],
  {
    tagIds: ['staging'],
  },
)

beforeEach(() => {
  vi.clearAllMocks()
  useHostStore.setState({
    folders: [
      { id: 'homelab', name: 'Homelab', parentId: null, order: 0 },
      { id: 'work', name: 'Work', parentId: null, order: 1 },
      { id: 'servers', name: 'Servers', parentId: 'work', order: 0 },
    ],
    hosts: [pc, nas, win, bastion, build],
    logins: [],
    keys: [],
    tags: [
      {
        id: 'staging',
        name: 'staging',
        color: 'amber',
        showColor: true,
        askBeforeClose: false,
        askBeforePaste: false,
        terminalColors: null,
      },
    ],
    shells: [{ name: 'PowerShell', program: 'pwsh.exe' }],
    loaded: true,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
  useSettingsStore.setState({ settings: { hostsView: 'cards' } as never, update } as never)
})

const cardNames = () =>
  screen
    .getAllByRole('article')
    .map((a) => within(a).getByRole('heading', { level: 3 }).textContent)

test('cards are grouped under their folder path, with hosts without a folder last', () => {
  render(<HostsPage />)
  const headings = screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)
  expect(headings).toEqual(['Homelab', 'Work', 'Work / Servers'])
  expect(cardNames()).toEqual(['This computer', 'nas.local', 'win-dev', 'build-01', 'bastion'])
})

test('This computer comes first with its shells', async () => {
  render(<HostsPage />)
  const first = screen.getAllByRole('article')[0]
  expect(within(first).getByRole('heading')).toHaveTextContent('This computer')
  expect(await within(first).findByRole('button', { name: 'PowerShell' })).toBeInTheDocument()
})

test('the query filters by name, address and tag', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.type(screen.getByPlaceholderText('Search or connect'), 'staging')
  expect(cardNames()).toEqual(['build-01'])
  await user.clear(screen.getByPlaceholderText('Search or connect'))
  await user.type(screen.getByPlaceholderText('Search or connect'), '192.168')
  expect(cardNames()).toEqual(['nas.local'])
})

test('the Files chip keeps hosts that have a files connection', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  const chips = screen.getByRole('group', { name: 'Filter by type' })
  await user.click(within(chips).getByRole('button', { name: 'Files' }))
  expect(cardNames()).toEqual(['nas.local'])
})

test('a selected folder shows its hosts and those of its subfolders', () => {
  useHostsUiStore.setState({ folderId: 'work' })
  render(<HostsPage />)
  expect(cardNames()).toEqual(['win-dev', 'build-01'])
})

test('the list view is remembered', async () => {
  const user = userEvent.setup()
  const { rerender } = render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'List' }))
  expect(update).toHaveBeenCalledWith({ hostsView: 'list' })
  act(() => useSettingsStore.setState({ settings: { hostsView: 'list' } as never }))
  rerender(<HostsPage />)
  expect(screen.queryAllByRole('article')).toHaveLength(0)
  expect(screen.getByRole('row', { name: /win-dev/ })).toBeInTheDocument()
})

test('a connection button opens that connection', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  const card = screen.getAllByRole('article')[1]
  await user.click(within(card).getByRole('button', { name: 'Files' }))
  expect(openConnection).toHaveBeenCalledWith(nas, nas.connections[1])
})

test('the address line names the jump host', () => {
  render(<HostsPage />)
  expect(screen.getByText('10.0.4.12 · via bastion')).toBeInTheDocument()
})

test('the first start page shows when only This computer exists', () => {
  useHostStore.setState({ hosts: [pc], folders: [] })
  render(<HostsPage />)
  expect(screen.getByRole('heading', { name: 'Add your first host' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: /Import from ~\/\.ssh\/config/ })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'PowerShell' })).toBeInTheDocument()
  expect(screen.queryByRole('heading', { name: 'Hosts' })).toBeNull()
})

test('del asks then deletes with undo', async () => {
  const deleteHost = vi.fn().mockResolvedValue('p9')
  useHostStore.setState({ deleteHost })
  const user = userEvent.setup()
  render(
    <>
      <HostsPage />
      <ConfirmDialog />
      <Notices />
    </>,
  )
  await user.click(screen.getByRole('button', { name: 'nas.local' }))
  await user.keyboard('{Delete}')
  expect(deleteHost).not.toHaveBeenCalled()
  await user.click(await screen.findByRole('button', { name: 'Delete' }))
  expect(deleteHost).toHaveBeenCalledWith('nas')
  expect(await screen.findByText('nas.local deleted')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument()
})
