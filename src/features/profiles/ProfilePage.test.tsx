import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { ProfilePage } from './ProfilePage'

const m = vi.hoisted(() => ({ setSecret: vi.fn(), deleteSecret: vi.fn() }))
vi.mock('@/bindings', () => ({
  commands: { setSecret: m.setSecret, deleteSecret: m.deleteSecret },
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn().mockResolvedValue('/home/me/.ssh/id_ed25519'),
}))

const saveProfile = vi.fn().mockResolvedValue(undefined)
const removeTab = vi.fn()

beforeEach(() => {
  vi.clearAllMocks()
  m.setSecret.mockResolvedValue({ status: 'ok', data: 'secret-123' })
  useProfileStore.setState({ groups: [], profiles: [], loaded: true, saveProfile } as never)
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null, removeTab } as never)
})

test('fills required fields and saves; then closes the tab', async () => {
  const user = userEvent.setup()
  render(<ProfilePage profileId={null} tabId="t1" />)
  await user.type(screen.getByLabelText('Name'), 'box')
  await user.type(screen.getByLabelText('Host'), '10.0.0.1')
  await user.type(screen.getByLabelText('Username'), 'root')
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveProfile).toHaveBeenCalledOnce()
  expect(saveProfile).toHaveBeenCalledWith(
    expect.objectContaining({ name: 'box', host: '10.0.0.1', username: 'root' }),
  )
  expect(removeTab).toHaveBeenCalledWith('t1')
})

test('populates fields when rendering an existing profile', () => {
  const profile = {
    id: 'p1',
    name: 'myserver',
    groupId: null,
    host: '192.168.1.1',
    port: 22,
    username: 'admin',
    authMethod: 'password',
    keyPath: null,
    secretId: null,
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
    jumpHostId: null,
    tunnels: [],
  }
  useProfileStore.setState({ groups: [], profiles: [profile], loaded: true, saveProfile } as never)
  render(<ProfilePage profileId="p1" tabId="t1" />)
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('myserver')
  expect((screen.getByLabelText('Host') as HTMLInputElement).value).toBe('192.168.1.1')
})

test('offers a group selector listing existing groups', () => {
  const group = {
    id: 'g1',
    name: 'Team',
    parentId: null,
    icon: { kind: 'builtin', name: 'folder' },
    order: 0,
  }
  useProfileStore.setState({
    groups: [group],
    profiles: [],
    loaded: true,
    saveProfile,
  } as never)
  render(<ProfilePage profileId={null} tabId="t1" />)
  expect(screen.getByRole('combobox', { name: 'Group' })).toBeInTheDocument()
})

test('per-profile font size override is saved as appearance', async () => {
  const user = userEvent.setup()
  render(<ProfilePage profileId={null} tabId="t1" />)
  await user.type(screen.getByLabelText('Name'), 'box')
  await user.type(screen.getByLabelText('Host'), 'h')
  await user.type(screen.getByLabelText('Username'), 'u')
  fireEvent.change(screen.getByLabelText('Font size override'), { target: { value: '18' } })
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveProfile).toHaveBeenCalledWith(
    expect.objectContaining({ appearance: expect.objectContaining({ fontSize: 18 }) }),
  )
})

async function chooseKeyAuth(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByLabelText('Auth method'))
  const items = await screen.findAllByText('Private key')
  await user.click(items[items.length - 1])
}

test('Browse fills a key path from the file picker', async () => {
  const user = userEvent.setup()
  render(<ProfilePage profileId={null} tabId="t1" />)
  await chooseKeyAuth(user)
  await user.click(await screen.findByRole('button', { name: /add key/i }))
  await user.click(screen.getByRole('button', { name: /browse/i }))
  await waitFor(() =>
    expect(screen.getByDisplayValue('/home/me/.ssh/id_ed25519')).toBeInTheDocument(),
  )
})

test('keys can be added and reordered, and the order is what gets saved', async () => {
  const user = userEvent.setup()
  render(<ProfilePage profileId={null} tabId="t1" />)
  await user.type(screen.getByLabelText('Name'), 'srv')
  await user.type(screen.getByLabelText('Host'), 'h')
  await user.type(screen.getByLabelText('Username'), 'u')
  await chooseKeyAuth(user)

  await user.click(await screen.findByRole('button', { name: /add key/i }))
  await user.type(screen.getByPlaceholderText('/home/me/.ssh/id_ed25519'), '/keys/first')
  await user.click(screen.getByRole('button', { name: /add key/i }))
  const paths = screen.getAllByPlaceholderText('/home/me/.ssh/id_ed25519')
  await user.type(paths[1], '/keys/second')

  await user.click(screen.getByRole('button', { name: 'Move key 2 up' }))
  await user.click(screen.getByRole('button', { name: /^save$/i }))

  await waitFor(() => expect(saveProfile).toHaveBeenCalled())
  const saved = saveProfile.mock.calls.at(-1)?.[0]
  expect(saved.keys.map((k: { path: string }) => k.path)).toEqual(['/keys/second', '/keys/first'])
})

test('editing a saved key-auth profile shows its keys', async () => {
  const profile = {
    id: 'p1',
    name: 'srv',
    groupId: null,
    host: 'h',
    port: 22,
    username: 'u',
    authMethod: 'key',
    keyPath: null,
    keys: [
      { path: '/home/me/.ssh/id_ed25519', secretId: null },
      { path: '/home/me/.ssh/id_rsa', secretId: 'vault-1' },
    ],
    secretId: null,
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
    jumpHostId: null,
    tunnels: [],
  }
  useProfileStore.setState({ groups: [], profiles: [profile], loaded: true, saveProfile } as never)
  render(<ProfilePage profileId="p1" tabId="t1" />)
  expect(await screen.findByDisplayValue('/home/me/.ssh/id_ed25519')).toBeInTheDocument()
  expect(screen.getByDisplayValue('/home/me/.ssh/id_rsa')).toBeInTheDocument()
  // A stored passphrase stays in the vault; the field only says one exists.
  expect(screen.getByPlaceholderText('passphrase unchanged')).toBeInTheDocument()
})

test('cancel closes the tab without saving', async () => {
  const user = userEvent.setup()
  render(<ProfilePage profileId={null} tabId="t1" />)
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(removeTab).toHaveBeenCalledWith('t1')
  expect(saveProfile).not.toHaveBeenCalled()
})
