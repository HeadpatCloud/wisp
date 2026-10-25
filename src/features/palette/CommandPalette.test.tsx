import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
import { useRdpProfileStore } from '@/stores/rdpProfileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { tabSecretIds, useSessionStore } from '@/stores/sessionStore'
import { useVncProfileStore } from '@/stores/vncProfileStore'
import { CommandPalette } from './CommandPalette'

const profile = (id: string, name: string, host: string) =>
  ({
    id,
    name,
    host,
    groupId: null,
    port: 22,
    username: 'u',
    authMethod: 'agent',
    keyPath: null,
    secretId: null,
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
    jumpHostId: null,
    tunnels: [],
  }) as never

beforeEach(() => {
  useProfileStore.setState({
    profiles: [profile('p1', 'web-01', '10.0.0.1'), profile('p2', 'db-02', '10.0.0.2')],
    groups: [],
    loaded: true,
  } as never)
  useS3ProfileStore.setState({ profiles: [], loaded: true } as never)
  useVncProfileStore.setState({ profiles: [], loaded: true })
  useRdpProfileStore.setState({ profiles: [], loaded: true })
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
})

test('lists profiles and actions, and filters by subsequence', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  expect(screen.getByText('web-01')).toBeInTheDocument()
  expect(screen.getByText('New local shell')).toBeInTheDocument()

  await user.type(screen.getByPlaceholderText('Search profiles and actions…'), 'wb01')
  expect(screen.getByText('web-01')).toBeInTheDocument()
  expect(screen.queryByText('db-02')).not.toBeInTheDocument()
})

test('Enter opens the highlighted profile as a session tab', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={onOpenChange} />)
  await user.type(screen.getByPlaceholderText('Search profiles and actions…'), 'db-02')
  await user.keyboard('{Enter}')

  const st = useSessionStore.getState()
  expect(st.tabs).toHaveLength(1)
  expect(st.tabs[0].kind).toBe('session')
  expect(Object.values(st.sessions)[0].profileId).toBe('p2')
  expect(onOpenChange).toHaveBeenCalledWith(false)
})

test('arrow keys move the highlight before running', async () => {
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  const input = screen.getByPlaceholderText('Search profiles and actions…')
  await user.type(input, 'sftp')
  await user.keyboard('{ArrowDown}{Enter}')

  const st = useSessionStore.getState()
  expect(st.tabs[0].kind).toBe('sftp')
})

const desk = {
  id: 'vnc-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 5901,
  username: 'faye',
  secretId: 'vault-1',
  icon: { kind: 'builtin' as const, name: 'server' },
  order: 0,
}

test('a saved VNC profile is listed under Connect with its address', () => {
  useVncProfileStore.setState({ profiles: [desk] })
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  const row = screen.getByText('VNC: desk').closest('button')
  expect(row).toHaveTextContent('10.0.0.5:5901')
  expect(row).toHaveTextContent('Connect')
})

test('running a VNC profile opens a new tab that holds the profile id and no secret', async () => {
  useVncProfileStore.setState({ profiles: [desk] })
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={onOpenChange} />)
  await user.type(screen.getByPlaceholderText('Search profiles and actions…'), 'vnc: desk')
  await user.keyboard('{Enter}')

  const { tabs, activeTabId } = useSessionStore.getState()
  expect(tabs).toEqual([
    {
      id: activeTabId,
      kind: 'vnc',
      title: 'desk',
      host: '10.0.0.5',
      port: 5901,
      username: 'faye',
      secretId: null,
      profileId: 'vnc-1',
    },
  ])
  expect(tabSecretIds(tabs[0])).toEqual([])
  expect(onOpenChange).toHaveBeenCalledWith(false)

  await user.click(screen.getByText('VNC: desk'))
  expect(useSessionStore.getState().tabs).toHaveLength(2)
})

const office = {
  id: 'rdp-1',
  name: 'office',
  host: '10.0.0.9',
  port: 3390,
  username: 'CORP\\faye',
  domain: 'CORP',
  secretId: 'vault-1',
  keyboardLayout: 0x407,
  icon: { kind: 'builtin' as const, name: 'server' },
  order: 0,
}

test('a saved RDP profile is listed under Connect with its address', () => {
  useRdpProfileStore.setState({ profiles: [office] })
  render(<CommandPalette open onOpenChange={vi.fn()} />)
  const row = screen.getByText('RDP: office').closest('button')
  expect(row).toHaveTextContent('10.0.0.9:3390')
  expect(row).toHaveTextContent('Connect')
})

test('running an RDP profile opens a new tab that holds the profile id and no secret', async () => {
  useRdpProfileStore.setState({ profiles: [office] })
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<CommandPalette open onOpenChange={onOpenChange} />)
  await user.type(screen.getByPlaceholderText('Search profiles and actions…'), 'rdp: office')
  await user.keyboard('{Enter}')

  const { tabs, activeTabId } = useSessionStore.getState()
  expect(tabs).toEqual([
    {
      id: activeTabId,
      kind: 'rdp',
      title: 'office',
      host: '10.0.0.9',
      port: 3390,
      username: 'CORP\\faye',
      domain: 'CORP',
      secretId: null,
      profileId: 'rdp-1',
      keyboardLayout: 0x407,
    },
  ])
  expect(tabSecretIds(tabs[0])).toEqual([])
  expect(onOpenChange).toHaveBeenCalledWith(false)

  await user.click(screen.getByText('RDP: office'))
  expect(useSessionStore.getState().tabs).toHaveLength(2)
})
