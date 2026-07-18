import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({
  events: {
    tunnelStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
    sshStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
    vncClipboard: { listen: vi.fn().mockResolvedValue(() => undefined) },
  },
}))
vi.mock('@/lib/ssh', () => ({
  connectSession: vi.fn().mockResolvedValue('sid'),
  disconnectSession: vi.fn().mockResolvedValue(undefined),
  writeSession: vi.fn(),
  resizeSession: vi.fn(),
  trustHostKey: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/tunnels', () => ({ startTunnel: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@/lib/local', () => ({
  listShells: vi.fn().mockResolvedValue([]),
  clearEditTemp: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/vault', () => ({
  setSecret: vi.fn().mockResolvedValue('vault-id'),
  deleteSecret: vi.fn().mockResolvedValue(undefined),
  vaultStatus: vi.fn().mockResolvedValue('unlocked'),
  vaultUnlock: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/theme', () => ({ watchSystemTheme: vi.fn().mockReturnValue(() => undefined) }))
vi.mock('@/stores/profileStore', () => ({
  useProfileStore: vi.fn(
    (sel: (s: { load: () => Promise<void>; profiles: unknown[] }) => unknown) =>
      sel({ load: vi.fn().mockResolvedValue(undefined), profiles: [] }),
  ),
}))
vi.mock('@/stores/settingsStore', () => ({
  useSettingsStore: vi.fn(
    (sel: (s: { load: () => Promise<void>; settings: { theme: string } }) => unknown) =>
      sel({ load: vi.fn().mockResolvedValue(undefined), settings: { theme: 'system' } }),
  ),
}))
vi.mock('@/features/profiles/ProfileTree', () => ({
  ProfileTree: () => <div data-testid="profile-tree" />,
}))
vi.mock('@/features/sessions/TabBar', () => ({
  TabBar: () => <div data-testid="tab-bar" />,
}))
vi.mock('@/features/sessions/PanesView', () => ({
  PanesView: ({ tab }: { tab: { id: string } }) => <div data-testid={`panesview-${tab.id}`} />,
}))
vi.mock('@/features/sessions/ViewHost', () => ({
  ViewHost: () => <div data-testid="view-host" />,
}))
vi.mock('@/features/welcome/WelcomePage', () => ({
  WelcomePage: () => <div data-testid="welcome-page" />,
}))

import { useSessionStore } from '@/stores/sessionStore'
import App from './App'

const tab1 = {
  id: 'tab-1',
  kind: 'session' as const,
  sessionIds: ['s1'],
  direction: 'horizontal' as const,
  activePaneId: 's1',
}
const tab2 = {
  id: 'tab-2',
  kind: 'session' as const,
  sessionIds: ['s2'],
  direction: 'horizontal' as const,
  activePaneId: 's2',
}

beforeEach(() => {
  vi.clearAllMocks()
  useSessionStore.setState({
    tabs: [tab1, tab2],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'host-1', status: 'connected', reconnectNonce: 0 },
      s2: { id: 's2', profileId: 'p2', title: 'host-2', status: 'connected', reconnectNonce: 0 },
    },
    activeTabId: 'tab-1',
  })
})

test('both session panes are mounted after render', () => {
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-1')).toBeInTheDocument()
  expect(screen.getByTestId('tabpane-tab-2')).toBeInTheDocument()
})

test('active pane wrapper lacks hidden class; inactive pane has it', () => {
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-1').className).not.toContain('hidden')
  expect(screen.getByTestId('tabpane-tab-2').className).toContain('hidden')
})

test('Home does not open a redundant tab when the welcome empty state is showing', () => {
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
  render(<App />)
  fireEvent.click(screen.getByRole('button', { name: 'Home' }))
  expect(useSessionStore.getState().tabs).toHaveLength(0)
})

test('switching active tab keeps both panes mounted and flips hidden class', () => {
  render(<App />)
  act(() => {
    useSessionStore.getState().setActiveTab('tab-2')
  })
  expect(screen.getByTestId('tabpane-tab-1')).toBeInTheDocument()
  expect(screen.getByTestId('tabpane-tab-2')).toBeInTheDocument()
  expect(screen.getByTestId('tabpane-tab-1').className).toContain('hidden')
  expect(screen.getByTestId('tabpane-tab-2').className).not.toContain('hidden')
})
