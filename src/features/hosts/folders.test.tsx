import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'

vi.mock('./openConnection', () => ({ openConnection: vi.fn(), openFiles: vi.fn() }))

import type { Connection, Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { HostsNav } from './HostsNav'
import { HostsPage } from './HostsPage'
import { folderDropMove, hostDropMoves, neighbourFolder } from './moves'
import { openConnection } from './openConnection'

function host(id: string, folderId: string | null, order: number): Host {
  return { id, name: id, address: `${id}.lan`, folderId, color: null, order, connections: [] }
}

const folders = [
  { id: 'homelab', name: 'Homelab', parentId: null, order: 0 },
  { id: 'work', name: 'Work', parentId: null, order: 1 },
  { id: 'servers', name: 'Servers', parentId: 'work', order: 0 },
]

describe('drop rules', () => {
  const hosts = [
    host('a', 'work', 0),
    host('b', 'work', 1),
    host('c', 'work', 2),
    host('x', null, 0),
  ]

  test('a host dropped on a folder goes to its end', () => {
    expect(hostDropMoves(hosts, 'x', { kind: 'folder', id: 'work' })).toEqual([
      { id: 'x', folderId: 'work', order: 3 },
    ])
    expect(hostDropMoves(hosts, 'a', { kind: 'folder', id: null })).toEqual([
      { id: 'a', folderId: null, order: 1 },
    ])
  })

  test('a host dropped between hosts numbers the folder again', () => {
    expect(hostDropMoves(hosts, 'x', { kind: 'before', hostId: 'b' })).toEqual([
      { id: 'a', folderId: 'work', order: 0 },
      { id: 'x', folderId: 'work', order: 1 },
      { id: 'b', folderId: 'work', order: 2 },
      { id: 'c', folderId: 'work', order: 3 },
    ])
    expect(hostDropMoves(hosts, 'c', { kind: 'after', hostId: 'a' })).toEqual([
      { id: 'a', folderId: 'work', order: 0 },
      { id: 'c', folderId: 'work', order: 1 },
      { id: 'b', folderId: 'work', order: 2 },
    ])
  })

  test('a folder dropped on a folder becomes its last child', () => {
    expect(folderDropMove(folders, 'homelab', { kind: 'folder', id: 'work' })).toEqual({
      id: 'homelab',
      parentId: 'work',
      order: 1,
    })
    expect(folderDropMove(folders, 'work', { kind: 'folder', id: 'work' })).toBeNull()
  })

  test('the neighbour folder follows the page order, with no folder last', () => {
    const order = ['homelab', 'work', 'servers']
    expect(neighbourFolder(order, 'homelab', 1)).toBe('work')
    expect(neighbourFolder(order, 'homelab', -1)).toBeUndefined()
    expect(neighbourFolder(order, 'servers', 1)).toBeNull()
    expect(neighbourFolder(order, null, -1)).toBe('servers')
    expect(neighbourFolder(order, null, 1)).toBeUndefined()
  })
})

const saveFolder = vi.fn().mockResolvedValue(undefined)
const saveHost = vi.fn().mockResolvedValue(undefined)
const moveHosts = vi.fn().mockResolvedValue(undefined)
const moveFolders = vi.fn().mockResolvedValue(undefined)

beforeEach(() => {
  vi.clearAllMocks()
  useHostStore.setState({
    folders,
    hosts: [host('nas', 'homelab', 0), host('pi', 'homelab', 1), host('win', 'work', 0)],
    logins: [],
    keys: [],
    tags: [],
    loaded: true,
    saveFolder,
    saveHost,
    moveHosts,
    moveFolders,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  useSettingsStore.setState({ settings: { hostsView: 'cards' } as never } as never)
})

afterEach(() => {
  vi.restoreAllMocks()
})

test('a new folder is saved with its name inside the selected folder', async () => {
  useHostsUiStore.setState({ folderId: 'work' })
  const user = userEvent.setup()
  render(<HostsNav />)
  await user.click(screen.getByRole('button', { name: 'New folder' }))
  await user.type(screen.getByLabelText('Folder name'), 'Lab{Enter}')
  expect(saveFolder).toHaveBeenCalledWith(
    expect.objectContaining({ name: 'Lab', parentId: 'work', order: 1 }),
  )
})

test('an empty name cancels the new folder', async () => {
  const user = userEvent.setup()
  render(<HostsNav />)
  await user.click(screen.getByRole('button', { name: 'New folder' }))
  await user.type(screen.getByLabelText('Folder name'), '{Enter}')
  expect(saveFolder).not.toHaveBeenCalled()
  expect(screen.queryByLabelText('Folder name')).toBeNull()
})

test('F2 on a folder renames it', async () => {
  const user = userEvent.setup()
  render(<HostsNav />)
  screen.getByRole('button', { name: 'Homelab 2' }).focus()
  await user.keyboard('{F2}')
  const field = screen.getByLabelText('Folder name')
  await user.clear(field)
  await user.type(field, 'Home{Enter}')
  expect(saveFolder).toHaveBeenCalledWith({ ...folders[0], name: 'Home' })
})

test('F2 renames the selected host', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'nas' }))
  await user.keyboard('{F2}')
  const field = screen.getByLabelText('Host name')
  await user.clear(field)
  await user.type(field, 'nas2{Enter}')
  expect(saveHost).toHaveBeenCalledWith(expect.objectContaining({ id: 'nas', name: 'nas2' }))
})

test('F2 on a nav folder leaves the selected host alone', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  const user = userEvent.setup()
  render(
    <>
      <HostsNav />
      <HostsPage />
    </>,
  )
  screen.getByRole('button', { name: 'Homelab 2' }).focus()
  await user.keyboard('{F2}')
  expect(screen.getByLabelText('Folder name')).toBeInTheDocument()
  expect(screen.queryByLabelText('Host name')).toBeNull()
})

test('Enter after an arrow key opens the host the arrow selected', async () => {
  const vnc = (id: string): Connection => ({
    id,
    label: null,
    address: null,
    kind: { type: 'vnc', port: 5900, username: null, secretId: null },
  })
  const nas = { ...host('nas', 'homelab', 0), connections: [vnc('nas-vnc')] }
  const pi = { ...host('pi', 'homelab', 1), connections: [vnc('pi-vnc')] }
  useHostStore.setState({ hosts: [nas, pi] })
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'nas' }))
  await user.keyboard('{ArrowDown}{Enter}')
  expect(openConnection).toHaveBeenCalledTimes(1)
  expect(openConnection).toHaveBeenCalledWith(pi, pi.connections[0])
})

test('arrow keys move the selection in the order the page shows', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'nas' }))
  await user.keyboard('{ArrowDown}')
  expect(useHostsUiStore.getState().selectedHostId).toBe('pi')
  await user.keyboard('{ArrowDown}')
  expect(useHostsUiStore.getState().selectedHostId).toBe('win')
  await user.keyboard('{ArrowUp}{ArrowUp}')
  expect(useHostsUiStore.getState().selectedHostId).toBe('nas')
})

test('Alt+Down moves the selected host to the end of the next folder', async () => {
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'pi' }))
  await user.keyboard('{Alt>}{ArrowDown}{/Alt}')
  expect(moveHosts).toHaveBeenCalledWith([{ id: 'pi', folderId: 'work', order: 1 }])
})

test('Alt+Up on a host without a folder moves it into the last folder above it', async () => {
  useHostStore.setState({ hosts: [host('nas', 'homelab', 0), host('loose', null, 0)] })
  const user = userEvent.setup()
  render(<HostsPage />)
  await user.click(screen.getByRole('button', { name: 'loose' }))
  await user.keyboard('{Alt>}{ArrowUp}{/Alt}')
  expect(moveHosts).toHaveBeenCalledWith([{ id: 'loose', folderId: 'servers', order: 0 }])
})

test('keys do nothing while another tab is in front', async () => {
  useHostsUiStore.setState({ selectedHostId: 'nas' })
  useSessionStore.setState({ activeTabId: 'some-tab' })
  render(<HostsPage />)
  act(() => {
    fireEvent.keyDown(window, { key: 'ArrowDown' })
  })
  expect(useHostsUiStore.getState().selectedHostId).toBe('nas')
})

test('a refused folder move shows the reason', async () => {
  moveFolders.mockRejectedValueOnce(new Error("The folder Work can't go inside its own subfolder."))
  render(<HostsNav />)
  const target = screen.getByRole('button', { name: 'Servers 0' })
  document.elementFromPoint = vi.fn().mockReturnValue(target)
  fireEvent.pointerDown(screen.getByRole('button', { name: 'Work 1' }), {
    button: 0,
    clientX: 0,
    clientY: 0,
  })
  fireEvent.pointerMove(window, { clientX: 10, clientY: 40 })
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: 10, clientY: 40 })
  })
  expect(moveFolders).toHaveBeenCalledWith([{ id: 'work', parentId: 'servers', order: 0 }])
  expect(await screen.findByRole('alert')).toHaveTextContent(
    "The folder Work can't go inside its own subfolder.",
  )
})
