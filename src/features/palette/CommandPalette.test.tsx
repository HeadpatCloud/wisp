import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
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
