import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/vault', () => ({ setSecret: vi.fn().mockResolvedValue('secret-1') }))

import type { Connection, Host } from '@/bindings'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Notices } from '@/components/Notices'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useNoticeStore } from '@/stores/noticeStore'
import { undoNotice } from './deletes'
import { HostEditorPage } from './HostEditorPage'

const ssh: Connection = {
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
const vnc: Connection = {
  id: 'nas-vnc',
  label: null,
  address: null,
  kind: { type: 'vnc', port: 5900, username: null, secretId: 'vnc-pw' },
}
const nas: Host = {
  id: 'nas',
  name: 'nas',
  address: 'nas.local',
  folderId: null,
  color: null,
  order: 0,
  connections: [ssh, vnc],
}
const box: Host = { ...nas, id: 'box', name: 'box', address: 'box.lan', connections: [] }

const saveHost = vi.fn()
const undoDelete = vi.fn().mockResolvedValue(undefined)
const purgeDeletes = vi.fn().mockResolvedValue(undefined)

beforeEach(() => {
  vi.clearAllMocks()
  saveHost.mockResolvedValue([])
  useNoticeStore.setState({ notices: [] })
  useHostStore.setState({
    folders: [],
    hosts: [nas, box],
    logins: [],
    keys: [],
    tags: [],
    shells: [],
    loaded: true,
    saveHost,
    undoDelete,
    purgeDeletes,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
})

function renderEditor() {
  render(
    <>
      <HostEditorPage hostId="nas" onClose={vi.fn()} />
      <Notices />
      <ConfirmDialog />
    </>,
  )
}

test('removing a connection and saving offers undo', async () => {
  saveHost.mockResolvedValue(['p1'])
  const user = userEvent.setup()
  renderEditor()
  await user.click(screen.getByRole('button', { name: 'More actions for VNC' }))
  await user.click(await screen.findByRole('menuitem', { name: 'Delete…' }))
  await user.click(await screen.findByRole('button', { name: 'Delete' }))
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveHost).toHaveBeenCalledWith(
    expect.objectContaining({ id: 'nas', connections: [ssh] }),
    [],
  )
  expect(await screen.findByText('VNC deleted')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Dismiss' }))
  expect(purgeDeletes).toHaveBeenCalledWith(['p1'])
  expect(undoDelete).not.toHaveBeenCalled()

  undoNotice('box deleted', 'p2')
  await user.click(await screen.findByRole('button', { name: 'Undo' }))
  expect(undoDelete).toHaveBeenCalledWith('p2')
  expect(purgeDeletes).not.toHaveBeenCalledWith(['p2'])
})

test('move to another host goes with the save', async () => {
  const user = userEvent.setup()
  renderEditor()
  await user.click(screen.getByRole('button', { name: 'More actions for VNC' }))
  await user.click(await screen.findByRole('menuitem', { name: 'Move to another host…' }))
  await user.click(await screen.findByRole('button', { name: 'Move' }))
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveHost).toHaveBeenCalledWith(
    expect.objectContaining({ id: 'nas', connections: [ssh] }),
    [{ connection: vnc, to: { kind: 'host', hostId: 'box' } }],
  )
})
