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
        kind: 'panes',
        paneIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
      {
        id: 't2',
        kind: 'panes',
        paneIds: ['s2'],
        direction: 'horizontal',
        activePaneId: 's2',
      },
    ],
    panes: {
      s1: {
        id: 's1',
        kind: 'ssh',
        connectionId: 'p1',
        title: 'web-01',
        status: 'connected',
        reconnectNonce: 0,
      },
      s2: {
        id: 's2',
        kind: 'ssh',
        connectionId: 'p2',
        title: 'db-01',
        status: 'connecting',
        reconnectNonce: 0,
      },
    },
    activeTabId: 't1',
  })
})

test('renders tabs and switches active on click', async () => {
  const user = userEvent.setup()
  render(<TabBar titleBar={false} onPalette={() => {}} />)
  expect(screen.getByText('web-01')).toBeInTheDocument()
  await user.click(screen.getByText('db-01'))
  expect(useSessionStore.getState().activeTabId).toBe('t2')
})

test('close removes the tab and its panes', async () => {
  const user = userEvent.setup()
  render(<TabBar titleBar={false} onPalette={() => {}} />)
  await user.click(screen.getByRole('button', { name: 'Close web-01' }))
  expect(useSessionStore.getState().tabs.find((t) => t.id === 't1')).toBeUndefined()
  expect(useSessionStore.getState().panes.s1).toBeUndefined()
})

test('the Hosts tab is always first and cannot be closed', () => {
  const { rerender } = render(<TabBar titleBar={false} onPalette={() => {}} />)
  const tabs = screen.getAllByRole('button').filter((b) => !b.getAttribute('aria-label'))
  expect(tabs[0]).toHaveTextContent('Hosts')
  expect(screen.queryByRole('button', { name: 'Close Hosts' })).not.toBeInTheDocument()

  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  rerender(<TabBar titleBar={false} onPalette={() => {}} />)
  expect(screen.getByRole('button', { name: 'Hosts' })).toBeInTheDocument()
})

test('clicking Hosts makes the Hosts tab active', async () => {
  const user = userEvent.setup()
  render(<TabBar titleBar={false} onPalette={() => {}} />)
  await user.click(screen.getByRole('button', { name: 'Hosts' }))
  expect(useSessionStore.getState().activeTabId).toBeNull()
})

test('closing the last tab returns to the Hosts tab', async () => {
  useSessionStore.setState({ tabs: [useSessionStore.getState().tabs[0]], activeTabId: 't1' })
  const user = userEvent.setup()
  render(<TabBar titleBar={false} onPalette={() => {}} />)
  await user.click(screen.getByRole('button', { name: 'Close web-01' }))
  expect(useSessionStore.getState().activeTabId).toBeNull()
})
