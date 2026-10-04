import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({}))
vi.mock('@/lib/transfer', () => ({
  pickExportPath: vi.fn().mockResolvedValue('C:/out.json'),
  exportBundle: vi.fn().mockResolvedValue({
    profiles: 2,
    secrets: 1,
    keyFiles: 1,
    warnings: ['Also exported jump host "b"'],
  }),
}))

import { exportBundle, pickExportPath } from '@/lib/transfer'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import { useVncProfileStore } from '@/stores/vncProfileStore'
import { ExportPage } from './ExportPage'

const base = {
  host: 'h',
  port: 22,
  username: 'u',
  authMethod: 'password',
  keyPath: null,
  keys: [],
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
  jumpHostId: null,
  tunnels: [],
}

const desk = {
  id: 'v',
  name: 'desk',
  host: 'h',
  port: 5900,
  username: null,
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
}

beforeEach(() => {
  vi.clearAllMocks()
  useProfileStore.setState({
    groups: [
      {
        id: 'g',
        name: 'Prod',
        parentId: null,
        icon: { kind: 'builtin', name: 'folder' },
        order: 0,
      },
    ],
    profiles: [
      { ...base, id: 'a', name: 'web', groupId: 'g' },
      { ...base, id: 'b', name: 'db', groupId: null },
    ],
  } as never)
  useSftpProfileStore.setState({ profiles: [] } as never)
  useS3ProfileStore.setState({ profiles: [] } as never)
  useVncProfileStore.setState({ profiles: [] } as never)
  useSessionStore.setState({ removeTab: vi.fn() } as never)
})

test('group checkbox selects its profiles and exports without a password', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('Prod'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: ['g'], profileIds: ['a'], sftpIds: [], s3Ids: [], vncIds: [] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
  expect(await screen.findByText(/Exported 2 profiles/)).toBeInTheDocument()
  expect(screen.getByText('Also exported jump host "b"')).toBeInTheDocument()
})

test('including secrets requires a matching password', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton).toBeDisabled()
  await user.type(screen.getByLabelText('Export password'), 'pw')
  await user.type(screen.getByLabelText('Confirm password'), 'pX')
  expect(exportButton).toBeDisabled()
  await user.clear(screen.getByLabelText('Confirm password'))
  await user.type(screen.getByLabelText('Confirm password'), 'pw')
  await user.click(exportButton)
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: [], profileIds: ['b'], sftpIds: [], s3Ids: [], vncIds: [] },
    { includeSecrets: true, includeKeys: false },
    'pw',
    'C:/out.json',
  )
})

test('ticking every member of a group checks the group', async () => {
  useProfileStore.setState({
    profiles: [
      { ...base, id: 'a', name: 'web', groupId: 'g' },
      { ...base, id: 'c', name: 'api', groupId: 'g' },
    ],
  } as never)
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  const group = screen.getByLabelText<HTMLInputElement>('Prod')
  await user.click(screen.getByLabelText('web'))
  expect(group).not.toBeChecked()
  expect(group.indeterminate).toBe(true)
  await user.click(screen.getByLabelText('api'))
  expect(group).toBeChecked()
  expect(group.indeterminate).toBe(false)
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: ['g'], profileIds: ['a', 'c'], sftpIds: [], s3Ids: [], vncIds: [] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
})

test('unticking the only member of a ticked group clears the group', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  const group = screen.getByLabelText<HTMLInputElement>('Prod')
  await user.click(group)
  await user.click(screen.getByLabelText('web'))
  expect(group).not.toBeChecked()
  expect(group.indeterminate).toBe(false)
  expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled()
})

test('ticking a group includes its empty subgroups', async () => {
  const icon = { kind: 'builtin', name: 'folder' }
  useProfileStore.setState({
    groups: [
      { id: 'g', name: 'Prod', parentId: null, icon, order: 0 },
      { id: 'e', name: 'Spare', parentId: 'g', icon, order: 0 },
    ],
  } as never)
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  const parent = screen.getByLabelText<HTMLInputElement>('Prod')
  const child = screen.getByLabelText<HTMLInputElement>('Spare')
  await user.click(parent)
  expect(child).toBeChecked()
  await user.click(child)
  expect(parent).not.toBeChecked()
  expect(parent.indeterminate).toBe(true)
  await user.click(child)
  expect(parent).toBeChecked()
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: ['g', 'e'], profileIds: ['a'], sftpIds: [], s3Ids: [], vncIds: [] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
})

test('a group whose parent is gone is listed at the top level', async () => {
  useProfileStore.setState({
    groups: [
      {
        id: 'g',
        name: 'Prod',
        parentId: 'missing',
        icon: { kind: 'builtin', name: 'folder' },
        order: 0,
      },
    ],
  } as never)
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('Prod'))
  expect(screen.getByLabelText('web')).toBeChecked()
})

test('the VNC section is listed only when there are VNC profiles', () => {
  render(<ExportPage tabId="t" />)
  expect(screen.queryByRole('heading', { name: 'VNC' })).not.toBeInTheDocument()
  act(() => useVncProfileStore.setState({ profiles: [desk] } as never))
  expect(screen.getByRole('heading', { name: 'VNC' })).toBeInTheDocument()
  expect(screen.getByLabelText('desk')).not.toBeChecked()
})

test('a VNC profile exports on its own', async () => {
  useVncProfileStore.setState({ profiles: [desk] } as never)
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton).toBeDisabled()
  await user.click(screen.getByLabelText('desk'))
  expect(exportButton).toBeEnabled()
  await user.click(exportButton)
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: [], profileIds: [], sftpIds: [], s3Ids: [], vncIds: ['v'] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
})

test('select all and select none cover the VNC profiles', async () => {
  useVncProfileStore.setState({ profiles: [desk] } as never)
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByRole('button', { name: 'Select all' }))
  expect(screen.getByLabelText('desk')).toBeChecked()
  await user.click(screen.getByRole('button', { name: 'Select none' }))
  expect(screen.getByLabelText('desk')).not.toBeChecked()
  await user.click(screen.getByRole('button', { name: 'Select all' }))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: ['g'], profileIds: ['a', 'b'], sftpIds: [], s3Ids: [], vncIds: ['v'] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
})

test('unticking the opt-in again exports without a password', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  await user.type(screen.getByLabelText('Export password'), 'pw')
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: [], profileIds: ['b'], sftpIds: [], s3Ids: [], vncIds: [] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
})

test('a failed export shows the message without an Error prefix', async () => {
  vi.mocked(exportBundle).mockRejectedValueOnce(new Error('io: denied'))
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(await screen.findByText('io: denied')).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
})

test('a failed or cancelled save dialog leaves the page usable', async () => {
  vi.mocked(pickExportPath).mockRejectedValueOnce(new Error('dialog unavailable'))
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  const exportButton = screen.getByRole('button', { name: 'Export' })
  await user.click(exportButton)
  expect(await screen.findByText('dialog unavailable')).toBeInTheDocument()
  expect(exportButton).toBeEnabled()
  vi.mocked(pickExportPath).mockResolvedValueOnce(null)
  await user.click(exportButton)
  expect(screen.queryByText('dialog unavailable')).not.toBeInTheDocument()
  expect(exportButton).toBeEnabled()
  expect(exportBundle).not.toHaveBeenCalled()
})

test('identical warnings are listed once', async () => {
  const consoleError = vi.spyOn(console, 'error')
  vi.mocked(exportBundle).mockResolvedValueOnce({
    profiles: 2,
    secrets: 0,
    keyFiles: 2,
    warnings: ["Couldn't read key file C:/k", "Couldn't read key file C:/k"],
  })
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(
    await screen.findByText('Exported 2 profiles (0 passwords, 2 key files).'),
  ).toBeInTheDocument()
  expect(screen.getAllByText("Couldn't read key file C:/k")).toHaveLength(1)
  expect(consoleError).not.toHaveBeenCalled()
  consoleError.mockRestore()
})

test('summary uses singular wording for counts of one', async () => {
  vi.mocked(exportBundle).mockResolvedValueOnce({
    profiles: 1,
    secrets: 1,
    keyFiles: 1,
    warnings: [],
  })
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(
    await screen.findByText('Exported 1 profile (1 password, 1 key file).'),
  ).toBeInTheDocument()
})

test('mismatch hint shows only while the passwords differ', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  await user.type(screen.getByLabelText('Export password'), 'pw')
  expect(screen.queryByText("Passwords don't match.")).not.toBeInTheDocument()
  await user.type(screen.getByLabelText('Confirm password'), 'pX')
  expect(screen.getByText("Passwords don't match.")).toBeInTheDocument()
  await user.clear(screen.getByLabelText('Confirm password'))
  await user.type(screen.getByLabelText('Confirm password'), 'pw')
  expect(screen.queryByText("Passwords don't match.")).not.toBeInTheDocument()
})
