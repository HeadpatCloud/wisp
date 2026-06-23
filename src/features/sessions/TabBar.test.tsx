import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test } from 'vitest'
import { useSessionStore } from '@/stores/sessionStore'
import { TabBar } from './TabBar'

beforeEach(() => {
  useSessionStore.setState({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
      {
        id: 't2',
        kind: 'session',
        sessionIds: ['s2'],
        direction: 'horizontal',
        activePaneId: 's2',
      },
    ],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'web-01', status: 'connected', reconnectNonce: 0 },
      s2: { id: 's2', profileId: 'p2', title: 'db-01', status: 'connecting', reconnectNonce: 0 },
    },
    activeTabId: 't1',
  })
})

test('renders tabs and switches active on click', async () => {
  const user = userEvent.setup()
  render(<TabBar />)
  expect(screen.getByText('web-01')).toBeInTheDocument()
  await user.click(screen.getByText('db-01'))
  expect(useSessionStore.getState().activeTabId).toBe('t2')
})

test('close removes the tab and its sessions', async () => {
  const user = userEvent.setup()
  render(<TabBar />)
  await user.click(screen.getByRole('button', { name: 'Close web-01' }))
  expect(useSessionStore.getState().tabs.find((t) => t.id === 't1')).toBeUndefined()
  expect(useSessionStore.getState().sessions.s1).toBeUndefined()
})

test('renders a view tab by its title', () => {
  useSessionStore.setState({
    tabs: [{ id: 'v1', kind: 'view', view: { kind: 'settings' }, title: 'Settings' }],
    sessions: {},
    activeTabId: 'v1',
  })
  render(<TabBar />)
  expect(screen.getByText('Settings')).toBeInTheDocument()
  expect(screen.getByLabelText('Close Settings')).toBeInTheDocument()
})
