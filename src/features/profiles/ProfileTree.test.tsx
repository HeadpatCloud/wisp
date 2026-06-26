import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
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
})

const noop = {
  onActivateProfile: vi.fn(),
  onNewProfile: vi.fn(),
  onNewVnc: vi.fn(),
  onNewFtp: vi.fn(),
  onNewS3: vi.fn(),
  onActivateS3: vi.fn(),
  onEditS3: vi.fn(),
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
