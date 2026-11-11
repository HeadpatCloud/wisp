import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import type { Connection, Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { tabSecretIds, useSessionStore } from '@/stores/sessionStore'
import { CommandPalette } from './CommandPalette'

const conn = (id: string, kind: Connection['kind']): Connection => ({
  id,
  label: null,
  address: null,
  kind,
})

const own = {
  kind: 'own',
  username: 'admin',
  authMethod: 'agent',
  keyIds: [],
  secretId: null,
} as const

function host(id: string, name: string, address: string, connections: Connection[]): Host {
  return { id, name, address, folderId: null, color: null, order: 0, connections }
}

const nas = host('h1', 'nas.local', '192.168.1.20', [
  conn('ssh-1', {
    type: 'ssh',
    port: 22,
    login: { ...own, keyIds: [] },
    jumpHostId: null,
    tunnels: [],
    appearance: null,
  }),
  conn('files-1', { type: 'sftp', own: null, startFolder: null }),
])
const desk = host('h2', 'desk', '10.0.0.5', [
  conn('vnc-1', { type: 'vnc', port: 5901, username: null, secretId: null }),
])

beforeEach(() => {
  useHostStore.setState({
    hosts: [nas, desk],
    logins: [],
    folders: [],
    keys: [],
    tags: [],
    loaded: true,
  })
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
})

test('lists every connection and the actions, and filters by subsequence', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  expect(screen.getByText('nas.local · SSH · admin')).toBeInTheDocument()
  expect(screen.getByText('nas.local · Files')).toBeInTheDocument()
  expect(screen.getByText('New local shell')).toBeInTheDocument()

  await user.type(screen.getByPlaceholderText('Search hosts and actions…'), 'dsk')
  expect(screen.getByText('desk · VNC')).toBeInTheDocument()
  expect(screen.queryByText('nas.local · Files')).not.toBeInTheDocument()
})

test('a local shell is listed by its shell name', () => {
  const pc = host('this-computer', 'This computer', '', [
    conn('pc-1', { type: 'local', program: 'pwsh.exe', startFolder: null }),
  ])
  useHostStore.setState({ hosts: [pc], shells: [{ name: 'PowerShell', program: 'pwsh.exe' }] })
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  expect(screen.getByText('This computer · PowerShell')).toBeInTheDocument()
})

test('Enter opens the highlighted connection by its id', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={onOpenChange} />)
  await user.type(screen.getByPlaceholderText('Search hosts and actions…'), 'nas ssh')
  await user.keyboard('{Enter}')

  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(1)
  expect(st.tabs[0].kind).toBe('panes')
  expect(Object.values(st.panes)[0]).toMatchObject({ kind: 'ssh', connectionId: 'ssh-1' })
  expect(onOpenChange).toHaveBeenCalledWith(false)
})

test('arrow keys move the highlight before running', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  await user.type(screen.getByPlaceholderText('Search hosts and actions…'), 'nas.local')
  await user.keyboard('{ArrowDown}{Enter}')
  expect(Object.values(useSessionStore.getState().panes)[0]).toMatchObject({
    kind: 'sftp',
    connectionId: 'files-1',
  })
})

test('a desktop connection shows its address and opens without a secret of its own', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  const row = screen.getByText('desk · VNC').closest('button')
  expect(row).toHaveTextContent('10.0.0.5')
  expect(row).toHaveTextContent('Connect')
  await user.click(screen.getByText('desk · VNC'))
  const tabs = useSessionStore.getState().tabs
  expect(tabs).toHaveLength(1)
  expect(tabs[0].kind === 'vnc' && tabs[0].connectionId).toBe('vnc-1')
  expect(tabSecretIds(tabs[0])).toEqual([])
})

test('actions passed in are listed and run', async () => {
  const run = vi.fn()
  const user = userEvent.setup()
  render(
    <CommandPalette
      open
      onOpenChange={vi.fn()}
      actions={[{ id: 'ftp', label: 'Connect to FTP once', run }]}
    />,
  )
  await user.click(screen.getByText('Connect to FTP once'))
  expect(run).toHaveBeenCalled()
})

test('ctrl_enter_opens_in_a_split', async () => {
  useSessionStore.getState().openSsh('ssh-1', 'nas.local')
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  await user.type(screen.getByPlaceholderText('Search hosts and actions…'), 'nas files')
  await user.keyboard('{Control>}{Enter}{/Control}')
  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(1)
  const tab = st.tabs[0]
  if (tab.kind !== 'panes') throw new Error('not a panes tab')
  expect(tab.paneIds.map((id) => st.panes[id].kind)).toEqual(['ssh', 'sftp'])
})

test('tab_shows_the_items_actions', async () => {
  useSessionStore.getState().openSsh('ssh-1', 'nas.local')
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={onOpenChange} />)
  await user.type(screen.getByPlaceholderText('Search hosts and actions…'), 'nas ssh')
  await user.keyboard('{Tab}')
  expect(screen.getByText('Open')).toBeInTheDocument()
  expect(screen.getByText('Open in split')).toBeInTheDocument()
  expect(screen.getByText('Edit nas.local')).toBeInTheDocument()
  await user.keyboard('{Escape}')
  expect(onOpenChange).not.toHaveBeenCalledWith(false)
  expect(screen.getByText('nas.local · SSH · admin')).toBeInTheDocument()
  await user.keyboard('{Tab}{ArrowDown}{Enter}')
  const st = useSessionStore.getState()
  const tab = st.tabs[0]
  if (tab.kind !== 'panes') throw new Error('not a panes tab')
  expect(tab.paneIds).toHaveLength(2)
  expect(onOpenChange).toHaveBeenCalledWith(false)
})

test('Tab on a row without actions keeps the keys in the search box', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  const input = screen.getByPlaceholderText('Search hosts and actions…')
  await user.type(input, 'new local')
  await user.keyboard('{Tab}')
  expect(input).toHaveFocus()
  await user.keyboard('{Enter}')
  expect(Object.values(useSessionStore.getState().panes)[0]).toMatchObject({ kind: 'local' })
})
