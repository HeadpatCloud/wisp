import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('./openConnection', () => ({ openConnection: vi.fn(), openFiles: vi.fn() }))

import type { Connection, Host, Login } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { reorderConnections } from './HostDetails'
import { HostsPage } from './HostsPage'
import { openConnection, openFiles } from './openConnection'

const saved: Login = { kind: 'saved', loginId: 'homelab' }

const c = (id: string, kind: Connection['kind']): Connection => ({
  id,
  label: null,
  address: null,
  kind,
})

const sshConn = c('nas-ssh', {
  type: 'ssh',
  port: 22,
  login: saved,
  jumpHostId: null,
  tunnels: [],
  appearance: null,
})
const files = c('nas-files', { type: 'sftp', own: null, startFolder: null })
const vnc = c('nas-vnc', { type: 'vnc', port: 5900, username: null, secretId: null })
const nas: Host = {
  id: 'nas',
  name: 'nas.local',
  address: '192.168.1.20',
  folderId: 'homelab',
  color: null,
  order: 0,
  connections: [sshConn, files, vnc],
}
const saveHost = vi.fn().mockResolvedValue(undefined)

beforeEach(() => {
  vi.clearAllMocks()
  useHostStore.setState({
    folders: [{ id: 'homelab', name: 'Homelab', parentId: null, order: 0 }],
    hosts: [nas],
    logins: [
      {
        id: 'homelab',
        name: 'homelab',
        username: 'admin',
        authMethod: 'key',
        keyIds: [],
        secretId: null,
      },
    ],
    keys: [],
    tags: [],
    loaded: true,
    saveHost,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
  useSettingsStore.setState({ settings: { hostsView: 'cards' } as never } as never)
})

const panel = () => screen.queryByRole('complementary', { name: 'Selected host' })

test('clicking a card selects it and opens the details', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  expect(panel()).toBeNull()
  await user.click(screen.getByRole('button', { name: 'nas.local' }))
  expect(useHostsUiStore.getState().selectedHostId).toBe('nas')
  const details = panel()
  expect(details).not.toBeNull()
  expect(within(details as HTMLElement).getByText('Homelab · 192.168.1.20')).toBeInTheDocument()
  expect(within(details as HTMLElement).getByText('Login "homelab" · key')).toBeInTheDocument()
  expect(within(details as HTMLElement).getByText('Same login as SSH')).toBeInTheDocument()
  expect(within(details as HTMLElement).getAllByText('Main')).toHaveLength(1)
})

test('a local shell row shows the shell name', () => {
  const pc: Host = {
    ...nas,
    id: 'this-computer',
    name: 'This computer',
    connections: [c('pc-1', { type: 'local', program: 'pwsh.exe', startFolder: null })],
  }
  useHostStore.setState({
    hosts: [pc, nas],
    shells: [{ name: 'PowerShell', program: 'pwsh.exe' }],
  })
  useHostsUiStore.setState({ selectedHostId: 'this-computer' })
  render(<HostsPage />)
  expect(within(panel() as HTMLElement).getByText('PowerShell')).toBeInTheDocument()
})

test('Enter on the selected host opens its main connection', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'nas.local' }))
  await user.keyboard('{Enter}')
  expect(openConnection).toHaveBeenCalledWith(nas, sshConn)
})

test('a double click opens the main connection', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.dblClick(screen.getByRole('button', { name: 'nas.local' }))
  expect(openConnection).toHaveBeenCalledWith(nas, sshConn)
})

test('Escape closes the details', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'nas.local' }))
  await user.keyboard('{Escape}')
  expect(panel()).toBeNull()
  expect(useHostsUiStore.getState().selectedHostId).toBeNull()
})

test('each connection row connects or opens it', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  const user = userEvent.setup()
  render(<HostsPage />)
  const details = panel() as HTMLElement
  await user.click(within(details).getByRole('button', { name: 'Open Files' }))
  expect(openConnection).toHaveBeenCalledWith(nas, files)
  await user.click(within(details).getByRole('button', { name: 'Connect VNC' }))
  expect(openConnection).toHaveBeenCalledWith(nas, vnc)
})

test('an SSH row offers to open files over it', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(within(panel() as HTMLElement).getByRole('button', { name: 'Open files' }))
  expect(openFiles).toHaveBeenCalledWith(nas, 'nas-ssh')
})

test('reordering puts the moved connection in its new place', () => {
  expect(reorderConnections(nas, 2, 0).connections.map((x) => x.id)).toEqual([
    'nas-vnc',
    'nas-ssh',
    'nas-files',
  ])
  expect(reorderConnections(nas, 0, 2).connections.map((x) => x.id)).toEqual([
    'nas-files',
    'nas-vnc',
    'nas-ssh',
  ])
})

test('Alt+arrow on a drag handle moves the connection and saves the host', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  const user = userEvent.setup()
  render(<HostsPage />)
  within(panel() as HTMLElement)
    .getByRole('button', { name: 'Drag to reorder VNC' })
    .focus()
  await user.keyboard('{Alt>}{ArrowUp}{/Alt}')
  expect(saveHost).toHaveBeenCalledWith({ ...nas, connections: [sshConn, vnc, files] })
})

test('dragging a connection row by its handle reorders the connections', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  const { fireEvent, act } = await import('@testing-library/react')
  render(<HostsPage />)
  const details = panel() as HTMLElement
  const rows = within(details).getAllByRole('listitem')
  document.elementFromPoint = vi.fn().mockReturnValue(rows[0])
  fireEvent.pointerDown(within(details).getByRole('button', { name: 'Drag to reorder VNC' }), {
    button: 0,
    clientX: 0,
    clientY: 100,
  })
  fireEvent.pointerMove(window, { clientX: 0, clientY: 10 })
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: 0, clientY: 10 })
  })
  expect(saveHost).toHaveBeenCalledWith({ ...nas, connections: [vnc, sshConn, files] })
})
