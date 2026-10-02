import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const remote = vi.hoisted(() => ({
  props: [] as { tabId: string; driver: unknown; active: boolean }[],
}))
vi.mock('@/bindings', () => ({
  events: {
    tunnelStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
    sshStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
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
vi.mock('@/features/remote/RemoteDesktopView', () => ({
  RemoteDesktopView: (props: { tabId: string; driver: unknown; active: boolean }) => {
    remote.props.push(props)
    return <div data-testid={`remote-${props.tabId}`} />
  },
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
  remote.props.length = 0
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

const importTab = {
  id: 'tab-import',
  kind: 'view' as const,
  view: { kind: 'transfer-import' as const, path: 'C:/a.json' },
  title: 'Import profiles',
}
const settingsTab = {
  id: 'tab-settings',
  kind: 'view' as const,
  view: { kind: 'settings' as const },
  title: 'Settings',
}

test('an import review tab stays mounted while another tab is active', () => {
  useSessionStore.setState({ tabs: [tab1, importTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-import').className).toContain('hidden')
  act(() => {
    useSessionStore.getState().setActiveTab('tab-import')
  })
  expect(screen.getByTestId('tabpane-tab-import').className).not.toContain('hidden')
})

test('other view tabs are only mounted while active', () => {
  useSessionStore.setState({ tabs: [tab1, settingsTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.queryByTestId('view-host')).not.toBeInTheDocument()
  act(() => {
    useSessionStore.getState().setActiveTab('tab-settings')
  })
  expect(screen.getByTestId('view-host')).toBeInTheDocument()
})

test('an active import review tab renders a single view host', () => {
  useSessionStore.setState({ tabs: [tab1, importTab], activeTabId: 'tab-import' })
  render(<App />)
  expect(screen.getAllByTestId('view-host')).toHaveLength(1)
  expect(screen.getByTestId('tabpane-tab-import')).toContainElement(screen.getByTestId('view-host'))
})

test('closing an import review tab unmounts it', () => {
  useSessionStore.setState({ tabs: [tab1, importTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-import')).toBeInTheDocument()
  act(() => {
    useSessionStore.getState().removeTab('tab-import')
  })
  expect(screen.queryByTestId('tabpane-tab-import')).not.toBeInTheDocument()
  expect(screen.queryByTestId('view-host')).not.toBeInTheDocument()
})

test('an export tab stays mounted while another tab is active', () => {
  const exportTab = {
    id: 'tab-export',
    kind: 'view' as const,
    view: { kind: 'transfer-export' as const },
    title: 'Export profiles',
  }
  useSessionStore.setState({ tabs: [tab1, exportTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-export').className).toContain('hidden')
  expect(screen.getByTestId('tabpane-tab-export')).toContainElement(screen.getByTestId('view-host'))
})

const vncTab = {
  id: 'tab-vnc',
  kind: 'vnc' as const,
  title: 'h:5900',
  host: 'h',
  port: 5900,
  secretId: 's1',
}

test('a VNC tab renders the remote view in its pane and tells it when it is active', () => {
  useSessionStore.setState({ tabs: [tab1, vncTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-vnc')).toContainElement(
    screen.getByTestId('remote-tab-vnc'),
  )
  expect(screen.getByTestId('tabpane-tab-vnc').className).toContain('hidden')
  expect(remote.props.at(-1)).toMatchObject({ tabId: 'tab-vnc', active: false })
  act(() => {
    useSessionStore.getState().setActiveTab('tab-vnc')
  })
  expect(screen.getByTestId('tabpane-tab-vnc').className).not.toContain('hidden')
  expect(remote.props.at(-1)).toMatchObject({ tabId: 'tab-vnc', active: true })
})

test('re-rendering the app keeps the driver of a VNC tab', () => {
  useSessionStore.setState({ tabs: [vncTab], activeTabId: 'tab-vnc' })
  const { rerender } = render(<App />)
  const { driver } = remote.props[0]
  expect(typeof (driver as { open: unknown }).open).toBe('function')
  rerender(<App />)
  act(() => {
    useSessionStore.setState({ tabs: [vncTab, tab1], activeTabId: 'tab-1' })
  })
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
})

test('a VNC tab for another target gets a driver of its own', () => {
  const other = { ...vncTab, id: 'tab-vnc-2', port: 5901 }
  useSessionStore.setState({ tabs: [vncTab, other], activeTabId: 'tab-vnc' })
  render(<App />)
  const first = remote.props.find((p) => p.tabId === 'tab-vnc')
  const second = remote.props.find((p) => p.tabId === 'tab-vnc-2')
  expect(first?.driver).toBeDefined()
  expect(second?.driver).toBeDefined()
  expect(second?.driver).not.toBe(first?.driver)
})
