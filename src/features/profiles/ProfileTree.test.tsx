import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import type { VncProfile } from '@/bindings'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import { useVncProfileStore } from '@/stores/vncProfileStore'
import { ProfileTree } from './ProfileTree'

const baseProfile = {
  id: 'p1',
  name: 'web-01',
  groupId: 'g1',
  host: '10.0.0.1',
  port: 22,
  username: 'root',
  authMethod: 'key',
  keyPath: null,
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
  jumpHostId: null,
  tunnels: [],
} as never

beforeEach(() => {
  useProfileStore.setState({
    groups: [
      {
        id: 'g1',
        name: 'Prod',
        parentId: null,
        icon: { kind: 'builtin', name: 'cloud' },
        order: 0,
      } as never,
    ],
    profiles: [baseProfile],
    loaded: true,
  })
  useVncProfileStore.setState({ profiles: [], loaded: true })
})

const noop = {
  onActivateProfile: vi.fn(),
  onNewProfile: vi.fn(),
  onNewVnc: vi.fn(),
  onNewVncProfile: vi.fn(),
  onActivateVnc: vi.fn(),
  onEditVnc: vi.fn(),
  onNewFtp: vi.fn(),
  onNewS3: vi.fn(),
  onActivateS3: vi.fn(),
  onEditS3: vi.fn(),
  onNewSftpProfile: vi.fn(),
  onActivateSftpProfile: vi.fn(),
  onEditSftpProfile: vi.fn(),
  onNewLocalShell: vi.fn(),
  onNewSftp: vi.fn(),
  onOpenSftpPicker: vi.fn(),
  onNewGroup: vi.fn(),
  onEditGroup: vi.fn(),
  shells: [],
}

test('renders group and profile names', () => {
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  expect(screen.getByText('Prod')).toBeInTheDocument()
  expect(screen.getByText('web-01')).toBeInTheDocument()
})

test('search filters profiles by name', async () => {
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  await user.type(screen.getByPlaceholderText('Search...'), 'nomatch')
  expect(screen.queryByText('web-01')).not.toBeInTheDocument()
})

test('double-click a profile triggers activate', async () => {
  const onActivateProfile = vi.fn()
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onActivateProfile={onActivateProfile} onEditProfile={vi.fn()} />)
  await user.dblClick(screen.getByText('web-01'))
  expect(onActivateProfile).toHaveBeenCalledWith(expect.objectContaining({ id: 'p1' }))
})

test('right-click Duplicate clones the profile and opens the copy in the editor', async () => {
  const saveProfile = vi.fn().mockResolvedValue(undefined)
  const onEditProfile = vi.fn()
  useProfileStore.setState({ saveProfile } as never)
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditProfile={onEditProfile} />)
  await user.pointer({ keys: '[MouseRight]', target: screen.getByText('web-01') })
  await user.click(await screen.findByText('Duplicate'))
  expect(saveProfile).toHaveBeenCalledWith(expect.objectContaining({ name: 'web-01 (copy)' }))
  await waitFor(() =>
    expect(onEditProfile).toHaveBeenCalledWith(expect.objectContaining({ name: 'web-01 (copy)' })),
  )
})

test('right-click Delete removes the profile', async () => {
  const removeProfile = vi.fn()
  useProfileStore.setState({ removeProfile } as never)
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  await user.pointer({ keys: '[MouseRight]', target: screen.getByText('web-01') })
  await user.click(await screen.findByText('Delete'))
  expect(removeProfile).toHaveBeenCalledWith('p1')
})

const vnc = (id: string, name: string, host: string): VncProfile => ({
  id,
  name,
  host,
  port: 5900,
  username: null,
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
})
const desk = vnc('vnc-1', 'desk', '10.0.0.5')
const mac = vnc('vnc-2', 'Office Mac', 'mac.example.com')

test('saved VNC profiles are listed under their own heading with name and host', () => {
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  expect(screen.queryByText('VNC')).not.toBeInTheDocument()
  act(() => useVncProfileStore.setState({ profiles: [desk, mac] }))
  expect(screen.getByText('VNC')).toBeInTheDocument()
  for (const text of ['desk', '10.0.0.5', 'Office Mac', 'mac.example.com']) {
    expect(screen.getByText(text)).toBeInTheDocument()
  }
})

test('double-click a VNC profile triggers activate', async () => {
  useVncProfileStore.setState({ profiles: [desk, mac] })
  const onActivateVnc = vi.fn()
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onActivateVnc={onActivateVnc} onEditProfile={vi.fn()} />)
  await user.click(screen.getByText('Office Mac'))
  expect(onActivateVnc).not.toHaveBeenCalled()
  await user.dblClick(screen.getByText('Office Mac'))
  expect(onActivateVnc).toHaveBeenCalledTimes(1)
  expect(onActivateVnc).toHaveBeenCalledWith(mac)
})

test.each([
  ['name', 'OFFICE', 'Office Mac', 'desk'],
  ['host', '10.0', 'desk', 'Office Mac'],
])('search filters VNC profiles by %s', async (_by, typed, shown, hidden) => {
  useVncProfileStore.setState({ profiles: [desk, mac] })
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  await user.type(screen.getByPlaceholderText('Search...'), typed)
  expect(screen.getByText(shown)).toBeInTheDocument()
  expect(screen.queryByText(hidden)).not.toBeInTheDocument()
})

test('right-click Edit on a VNC profile opens it for editing', async () => {
  useVncProfileStore.setState({ profiles: [desk, mac] })
  const onEditVnc = vi.fn()
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditVnc={onEditVnc} onEditProfile={vi.fn()} />)
  await user.pointer({ keys: '[MouseRight]', target: screen.getByText('desk') })
  await user.click(await screen.findByText('Edit'))
  expect(onEditVnc).toHaveBeenCalledWith(desk)
})

test('right-click Delete on a VNC profile removes it through the store', async () => {
  const remove = vi.fn()
  useVncProfileStore.setState({ profiles: [desk, mac], remove } as never)
  const user = userEvent.setup()
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  await user.pointer({ keys: '[MouseRight]', target: screen.getByText('desk') })
  await user.click(await screen.findByText('Delete'))
  expect(remove).toHaveBeenCalledTimes(1)
  expect(remove).toHaveBeenCalledWith('vnc-1')
})

test('the add menu offers a VNC profile next to the VNC quick connect', async () => {
  const onNewVncProfile = vi.fn()
  const onNewVnc = vi.fn()
  const user = userEvent.setup()
  render(
    <ProfileTree
      {...noop}
      onNewVncProfile={onNewVncProfile}
      onNewVnc={onNewVnc}
      onEditProfile={vi.fn()}
    />,
  )
  await user.click(screen.getByRole('button', { name: 'New connection' }))
  expect(await screen.findByText('VNC connection')).toBeInTheDocument()
  await user.click(screen.getByText('VNC profile'))
  expect(onNewVncProfile).toHaveBeenCalledTimes(1)
  expect(onNewVnc).not.toHaveBeenCalled()
})

test('the empty hint is not shown when a VNC profile is all there is', () => {
  const hint = 'No hosts yet. Use + to add a profile, or Import from ~/.ssh/config.'
  useProfileStore.setState({ groups: [], profiles: [] })
  useS3ProfileStore.setState({ profiles: [] })
  useSftpProfileStore.setState({ profiles: [] })
  render(<ProfileTree {...noop} onEditProfile={vi.fn()} />)
  expect(screen.getByText(hint)).toBeInTheDocument()
  act(() => useVncProfileStore.setState({ profiles: [desk] }))
  expect(screen.queryByText(hint)).not.toBeInTheDocument()
})
