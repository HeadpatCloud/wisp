import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const remote = vi.hoisted(() => ({
  props: [] as { tabId: string; driver: unknown; active: boolean }[],
}))
const desk = vi.hoisted(() => ({
  id: 'vnc-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 5901,
  username: 'faye',
  secretId: 'vault-desk',
  icon: { kind: 'builtin' as const, name: 'server' },
  order: 0,
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
vi.mock('@/lib/vnc', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/vnc')>()
  return { vncDriver: vi.fn(actual.vncDriver) }
})
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
  ProfileTree: (props: {
    onNewVnc: () => void
    onNewVncProfile: () => void
    onActivateVnc: (profile: typeof desk) => void
    onEditVnc: (profile: typeof desk) => void
  }) => (
    <div data-testid="profile-tree">
      <button type="button" onClick={props.onNewVnc}>
        New VNC
      </button>
      <button type="button" onClick={props.onNewVncProfile}>
        Add VNC profile
      </button>
      <button type="button" onClick={() => props.onActivateVnc(desk)}>
        Open desk
      </button>
      <button type="button" onClick={() => props.onEditVnc(desk)}>
        Edit desk
      </button>
    </div>
  ),
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

import { deleteSecret, setSecret } from '@/lib/vault'
import { vncDriver } from '@/lib/vnc'
import { tabSecretIds, useSessionStore } from '@/stores/sessionStore'
import { useVncProfileStore } from '@/stores/vncProfileStore'
import App from './App'

const loadVncProfiles = vi.fn()

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
  loadVncProfiles.mockResolvedValue(undefined)
  useVncProfileStore.setState({ profiles: [desk], loaded: true, load: loadVncProfiles })
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
  username: null,
  secretId: 's1',
  profileId: null,
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
  const tab = { ...vncTab, username: 'alice' }
  useSessionStore.setState({ tabs: [tab], activeTabId: 'tab-vnc' })
  const { rerender } = render(<App />)
  const { driver } = remote.props[0]
  expect(typeof (driver as { open: unknown }).open).toBe('function')
  rerender(<App />)
  act(() => {
    useSessionStore.setState({ tabs: [{ ...tab }, tab1], activeTabId: 'tab-1' })
  })
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
  expect(vi.mocked(vncDriver).mock.calls).toEqual([
    [{ host: 'h', port: 5900, username: 'alice', secretId: 's1', profileId: null }],
  ])
})

test('a VNC tab whose username changed gets a new driver', () => {
  useSessionStore.setState({ tabs: [vncTab], activeTabId: 'tab-vnc' })
  render(<App />)
  const { driver } = remote.props[0]
  act(() => {
    useSessionStore.setState({ tabs: [{ ...vncTab, username: 'alice' }] })
  })
  expect(remote.props.at(-1)?.driver).not.toBe(driver)
  expect(vncDriver).toHaveBeenLastCalledWith({
    host: 'h',
    port: 5900,
    username: 'alice',
    secretId: 's1',
    profileId: null,
  })
})

test('a VNC tab whose profile changed gets a new driver', () => {
  useSessionStore.setState({ tabs: [vncTab], activeTabId: 'tab-vnc' })
  render(<App />)
  const { driver } = remote.props[0]
  act(() => {
    useSessionStore.setState({ tabs: [{ ...vncTab, profileId: 'vnc-1' }] })
  })
  expect(remote.props.at(-1)?.driver).not.toBe(driver)
  expect(vncDriver).toHaveBeenLastCalledWith({
    host: 'h',
    port: 5900,
    username: null,
    secretId: 's1',
    profileId: 'vnc-1',
  })
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

async function connectVnc(username: string, password: string) {
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
  render(<App />)
  fireEvent.click(screen.getByRole('button', { name: 'New VNC' }))
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'mac' } })
  fireEvent.change(screen.getByLabelText('Username (optional)'), { target: { value: username } })
  fireEvent.change(screen.getByLabelText('Password'), { target: { value: password } })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }))
  })
  return useSessionStore.getState().tabs
}

test('the VNC dialog opens a quick-connect tab with the username and the stored password', async () => {
  const tabs = await connectVnc(' alice ', 'hunter2')
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(tabs).toEqual([
    {
      id: tabs[0].id,
      kind: 'vnc',
      title: 'mac:5900',
      host: 'mac',
      port: 5900,
      username: 'alice',
      secretId: 'vault-id',
      profileId: null,
    },
  ])
  expect(screen.getByTestId(`tabpane-${tabs[0].id}`)).toBeInTheDocument()
})

test.each(['', '   '])('the VNC dialog opens a tab without a username for "%s"', async (typed) => {
  const tabs = await connectVnc(typed, '')
  expect(setSecret).not.toHaveBeenCalled()
  expect(tabs).toHaveLength(1)
  expect(tabs[0]).toMatchObject({ kind: 'vnc', username: null, secretId: null, profileId: null })
})

test('closing a quick-connect VNC tab deletes its password', async () => {
  const tabs = await connectVnc('', 'hunter2')
  act(() => {
    useSessionStore.getState().removeTab(tabs[0].id)
  })
  expect(deleteSecret).toHaveBeenCalledWith('vault-id')
})

test('the saved VNC profiles are loaded at start', () => {
  render(<App />)
  expect(loadVncProfiles).toHaveBeenCalledTimes(1)
})

function openDesk() {
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null })
  const { rerender } = render(<App />)
  fireEvent.click(screen.getByRole('button', { name: 'Open desk' }))
  return { tabs: useSessionStore.getState().tabs, rerender }
}

const deskTarget = {
  host: '10.0.0.5',
  port: 5901,
  username: 'faye',
  secretId: null,
  profileId: 'vnc-1',
}

test('opening a saved VNC profile opens a tab with the profile id and without its secret', () => {
  const { tabs } = openDesk()
  expect(tabs).toEqual([{ id: tabs[0].id, kind: 'vnc', title: 'desk', ...deskTarget }])
  expect(tabSecretIds(tabs[0])).toEqual([])
  expect(vi.mocked(vncDriver).mock.calls).toEqual([[deskTarget]])
})

test('opening a saved VNC profile again opens another tab', () => {
  openDesk()
  fireEvent.click(screen.getByRole('button', { name: 'Open desk' }))
  const { tabs } = useSessionStore.getState()
  expect(tabs).toHaveLength(2)
  expect(tabs[1]).toMatchObject({ kind: 'vnc', ...deskTarget })
  expect(tabs[1].id).not.toBe(tabs[0].id)
})

test('closing a tab of a saved VNC profile deletes no password', () => {
  const { tabs } = openDesk()
  act(() => {
    useSessionStore.getState().removeTab(tabs[0].id)
  })
  expect(useSessionStore.getState().tabs).toHaveLength(0)
  expect(deleteSecret).not.toHaveBeenCalled()
})

test('a profile tab keeps its driver when the password of the profile is replaced', () => {
  const { rerender } = openDesk()
  const { driver } = remote.props[0]
  act(() => {
    useVncProfileStore.setState({ profiles: [{ ...desk, secretId: 'vault-replaced' }] })
  })
  rerender(<App />)
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
  expect(vi.mocked(vncDriver).mock.calls).toEqual([[deskTarget]])
})

test('the tree opens the VNC profile dialog for a saved profile and for a new one', () => {
  render(<App />)
  fireEvent.click(screen.getByRole('button', { name: 'Edit desk' }))
  expect(screen.getByText('Edit VNC profile')).toBeInTheDocument()
  expect(screen.getByLabelText('Host')).toHaveValue('10.0.0.5')
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(screen.queryByText('Edit VNC profile')).not.toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: 'Add VNC profile' }))
  expect(screen.getByText('New VNC profile')).toBeInTheDocument()
  expect(screen.getByLabelText('Host')).toHaveValue('')
})
