import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const remote = vi.hoisted(() => ({
  props: [] as { tabId: string; driver: unknown; active: boolean }[],
}))
const takeMigrationReport = vi.hoisted(() => vi.fn())
vi.mock('@/bindings', () => ({
  commands: { takeMigrationReport },
  events: {
    tunnelStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
    sshStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
  },
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: vi.fn(),
  message: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
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
vi.mock('@/lib/rdp', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/rdp')>()
  return { rdpDriver: vi.fn(actual.rdpDriver) }
})
vi.mock('@/stores/settingsStore', () => ({
  useSettingsStore: vi.fn(
    (sel: (s: { load: () => Promise<void>; settings: { theme: string } }) => unknown) =>
      sel({ load: vi.fn().mockResolvedValue(undefined), settings: { theme: 'system' } }),
  ),
}))
vi.mock('@/features/sessions/TabBar', () => ({
  TabBar: () => <div data-testid="tab-bar" />,
}))
vi.mock('@/features/sessions/PanesView', () => ({
  PanesView: ({ tab }: { tab: { id: string } }) => <div data-testid={`panesview-${tab.id}`} />,
}))
vi.mock('@/features/sessions/PaneLayer', () => ({ PaneLayer: () => null }))
vi.mock('@/features/hosts/HostsTab', () => ({
  HostsTab: () => <div data-testid="hosts-tab" />,
}))
vi.mock('@/features/remote/RemoteDesktopView', () => ({
  RemoteDesktopView: (props: { tabId: string; driver: unknown; active: boolean }) => {
    remote.props.push(props)
    return <div data-testid={`remote-${props.tabId}`} />
  },
}))

import type { Connection, Host } from '@/bindings'
import { openConnection } from '@/features/hosts/openConnection'
import { rdpDriver } from '@/lib/rdp'
import { deleteSecret, setSecret } from '@/lib/vault'
import { vncDriver } from '@/lib/vnc'
import { useHostStore } from '@/stores/hostStore'
import { useNoticeStore } from '@/stores/noticeStore'
import { type RdpTab, tabSecretIds, useSessionStore, type VncTab } from '@/stores/sessionStore'
import App from './App'

const loadHosts = vi.fn()

const deskConn: Connection = {
  id: 'vnc-1',
  label: null,
  address: null,
  kind: { type: 'vnc', port: 5901, username: 'faye', secretId: 'vault-desk' },
}
const deskHost: Host = {
  id: 'h-desk',
  name: 'desk',
  address: '10.0.0.5',
  folderId: null,
  color: null,
  order: 0,
  connections: [deskConn],
}
const officeConn: Connection = {
  id: 'rdp-1',
  label: null,
  address: null,
  kind: {
    type: 'rdp',
    port: 3390,
    login: {
      kind: 'own',
      username: 'CORP\\faye',
      authMethod: 'password',
      keyIds: [],
      secretId: 'vault-office',
    },
    domain: 'CORP',
    keyboardLayout: 0x407,
  },
}
const officeHost: Host = {
  ...deskHost,
  id: 'h-office',
  name: 'office',
  address: '10.0.0.9',
  connections: [officeConn],
}
const thisComputer: Host = {
  ...deskHost,
  id: 'this-computer',
  name: 'This computer',
  address: '',
  connections: [
    {
      id: 'pc-1',
      label: null,
      address: null,
      kind: { type: 'local', program: 'pwsh.exe', startFolder: null },
    },
  ],
}

const tab1 = {
  id: 'tab-1',
  kind: 'panes' as const,
  paneIds: ['s1'],
  direction: 'horizontal' as const,
  activePaneId: 's1',
}
const tab2 = {
  id: 'tab-2',
  kind: 'panes' as const,
  paneIds: ['s2'],
  direction: 'horizontal' as const,
  activePaneId: 's2',
}

beforeEach(() => {
  vi.clearAllMocks()
  useNoticeStore.setState({ notices: [] })
  remote.props.length = 0
  loadHosts.mockResolvedValue(undefined)
  takeMigrationReport.mockResolvedValue({ status: 'ok', data: null })
  useHostStore.setState({
    hosts: [thisComputer, deskHost, officeHost],
    folders: [],
    logins: [],
    keys: [],
    tags: [],
    loaded: true,
    load: loadHosts,
  })
  useSessionStore.setState({
    tabs: [tab1, tab2],
    panes: {
      s1: {
        id: 's1',
        kind: 'ssh',
        connectionId: 'p1',
        title: 'host-1',
        status: 'connected',
        reconnectNonce: 0,
      },
      s2: {
        id: 's2',
        kind: 'ssh',
        connectionId: 'p2',
        title: 'host-2',
        status: 'connected',
        reconnectNonce: 0,
      },
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

test('moving a tab leaves the tab views where they are', () => {
  render(<App />)
  const views = () => [...document.querySelectorAll('[data-testid^="tabpane-tab-"]')]
  const before = views()
  act(() => {
    useSessionStore.getState().moveTab('tab-1', 1)
  })
  expect(useSessionStore.getState().tabs.map((t) => t.id)).toEqual(['tab-2', 'tab-1'])
  expect(views()).toEqual(before)
})

const vncTab: VncTab = {
  id: 'tab-vnc',
  kind: 'vnc',
  title: 'h:5900',
  connectionId: null,
  once: { host: 'h', port: 5900, username: null, secretId: 's1' },
}
const vncOnce = { host: 'h', port: 5900, username: null, secretId: 's1' }

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
  const tab = { ...vncTab, once: { ...vncOnce, username: 'alice' } }
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

test('a VNC tab for another target gets a driver of its own', () => {
  const other = { ...vncTab, id: 'tab-vnc-2', once: { ...vncOnce, port: 5901 } }
  useSessionStore.setState({ tabs: [vncTab, other], activeTabId: 'tab-vnc' })
  render(<App />)
  const first = remote.props.find((p) => p.tabId === 'tab-vnc')
  const second = remote.props.find((p) => p.tabId === 'tab-vnc-2')
  expect(first?.driver).toBeDefined()
  expect(second?.driver).toBeDefined()
  expect(second?.driver).not.toBe(first?.driver)
})

async function connectVnc(username: string, password: string) {
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  render(<App />)
  fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true })
  fireEvent.click(screen.getByText('Connect to VNC once…'))
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
      connectionId: null,
      once: { host: 'mac', port: 5900, username: 'alice', secretId: 'vault-id' },
    },
  ])
  expect(screen.getByTestId(`tabpane-${tabs[0].id}`)).toBeInTheDocument()
})

test.each(['', '   '])('the VNC dialog opens a tab without a username for "%s"', async (typed) => {
  const tabs = await connectVnc(typed, '')
  expect(setSecret).not.toHaveBeenCalled()
  expect(tabs).toHaveLength(1)
  expect(tabs[0]).toMatchObject({
    kind: 'vnc',
    connectionId: null,
    once: { username: null, secretId: null },
  })
})

test('a VNC password that cannot be stored opens no tab and is reported', async () => {
  vi.mocked(setSecret).mockRejectedValueOnce(new Error('io: disk full'))
  const tabs = await connectVnc('', 'hunter2')
  expect(tabs).toEqual([])
  const alert = await screen.findByRole('alert')
  expect(alert).toHaveTextContent('Could not store the password')
  expect(alert).toHaveTextContent('Error: io: disk full')
  expect(screen.queryByLabelText('Password')).not.toBeInTheDocument()
})

test('closing a quick-connect VNC tab deletes its password', async () => {
  const tabs = await connectVnc('', 'hunter2')
  act(() => {
    useSessionStore.getState().removeTab(tabs[0].id)
  })
  expect(deleteSecret).toHaveBeenCalledWith('vault-id')
})

const rdpOnce = {
  host: 'pc',
  port: 3389,
  username: 'alice',
  domain: null,
  secretId: 's1',
  keyboardLayout: null,
}
const rdpTab: RdpTab = {
  id: 'tab-rdp',
  kind: 'rdp',
  title: 'pc:3389',
  connectionId: null,
  once: rdpOnce,
}

test('an RDP tab renders the remote view in its pane and tells it when it is active', () => {
  useSessionStore.setState({ tabs: [tab1, rdpTab], activeTabId: 'tab-1' })
  render(<App />)
  expect(screen.getByTestId('tabpane-tab-rdp')).toContainElement(
    screen.getByTestId('remote-tab-rdp'),
  )
  expect(screen.getByTestId('tabpane-tab-rdp').className).toContain('hidden')
  expect(remote.props.at(-1)).toMatchObject({ tabId: 'tab-rdp', active: false })
  act(() => {
    useSessionStore.getState().setActiveTab('tab-rdp')
  })
  expect(screen.getByTestId('tabpane-tab-rdp').className).not.toContain('hidden')
  expect(remote.props.at(-1)).toMatchObject({ tabId: 'tab-rdp', active: true })
})

test('re-rendering the app keeps the driver of an RDP tab', () => {
  const tab = { ...rdpTab, once: { ...rdpOnce, domain: 'CORP', keyboardLayout: 0x407 } }
  useSessionStore.setState({ tabs: [tab], activeTabId: 'tab-rdp' })
  const { rerender } = render(<App />)
  const { driver } = remote.props[0]
  expect(typeof (driver as { open: unknown }).open).toBe('function')
  rerender(<App />)
  act(() => {
    useSessionStore.setState({ tabs: [{ ...tab }, tab1], activeTabId: 'tab-1' })
  })
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
  expect(vi.mocked(rdpDriver).mock.calls).toEqual([
    [
      {
        host: 'pc',
        port: 3389,
        username: 'alice',
        domain: 'CORP',
        secretId: 's1',
        profileId: null,
        keyboardLayout: 0x407,
      },
    ],
  ])
})

test('an RDP tab for another target gets a driver of its own', () => {
  const other = { ...rdpTab, id: 'tab-rdp-2', once: { ...rdpOnce, port: 3390 } }
  useSessionStore.setState({ tabs: [rdpTab, other], activeTabId: 'tab-rdp' })
  render(<App />)
  const first = remote.props.find((p) => p.tabId === 'tab-rdp')
  const second = remote.props.find((p) => p.tabId === 'tab-rdp-2')
  expect(first?.driver).toBeDefined()
  expect(second?.driver).toBeDefined()
  expect(second?.driver).not.toBe(first?.driver)
})

async function connectRdp(fields: { username?: string; domain?: string; password?: string }) {
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  render(<App />)
  fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true })
  fireEvent.click(screen.getByText('Connect to RDP once…'))
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'pc' } })
  fireEvent.change(screen.getByLabelText('Username'), { target: { value: fields.username ?? '' } })
  fireEvent.change(screen.getByLabelText('Domain (optional)'), {
    target: { value: fields.domain ?? '' },
  })
  fireEvent.change(screen.getByLabelText('Password'), { target: { value: fields.password ?? '' } })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }))
  })
  return useSessionStore.getState().tabs
}

test('the RDP dialog opens a quick-connect tab with the stored password', async () => {
  const tabs = await connectRdp({ username: ' alice ', domain: ' CORP ', password: 'hunter2' })
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(tabs).toEqual([
    {
      id: tabs[0].id,
      kind: 'rdp',
      title: 'pc:3389',
      connectionId: null,
      once: {
        host: 'pc',
        port: 3389,
        username: 'alice',
        domain: 'CORP',
        secretId: 'vault-id',
        keyboardLayout: null,
      },
    },
  ])
  expect(screen.getByTestId(`tabpane-${tabs[0].id}`)).toBeInTheDocument()
  expect(screen.queryByLabelText('Password')).not.toBeInTheDocument()
})

test('the RDP dialog opens a tab without a domain or a password when none is typed', async () => {
  const tabs = await connectRdp({ username: 'alice', domain: '  ' })
  expect(setSecret).not.toHaveBeenCalled()
  expect(tabs).toHaveLength(1)
  expect(tabs[0]).toMatchObject({
    kind: 'rdp',
    connectionId: null,
    once: { domain: null, secretId: null },
  })
})

test('an RDP password that cannot be stored opens no tab and is reported', async () => {
  vi.mocked(setSecret).mockRejectedValueOnce(new Error('io: disk full'))
  const tabs = await connectRdp({ username: 'alice', password: 'hunter2' })
  expect(tabs).toEqual([])
  const alert = await screen.findByRole('alert')
  expect(alert).toHaveTextContent('Could not store the password')
  expect(alert).toHaveTextContent('Error: io: disk full')
  expect(screen.queryByLabelText('Password')).not.toBeInTheDocument()
})

test('closing a quick-connect RDP tab deletes its password', async () => {
  const tabs = await connectRdp({ username: 'alice', password: 'hunter2' })
  act(() => {
    useSessionStore.getState().removeTab(tabs[0].id)
  })
  expect(deleteSecret).toHaveBeenCalledWith('vault-id')
})

test('the migration notice shows once the vault is ready', async () => {
  takeMigrationReport.mockResolvedValue({ status: 'ok', data: { profiles: 3, hosts: 2 } })
  render(<App />)
  expect(await screen.findByText('3 profiles became 2 hosts')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'OK' }))
  expect(screen.queryByText('3 profiles became 2 hosts')).not.toBeInTheDocument()
})

test('no notice without a migration', async () => {
  render(<App />)
  await act(async () => {})
  expect(takeMigrationReport).toHaveBeenCalled()
  expect(screen.queryByText(/became/)).not.toBeInTheDocument()
})

test('the hosts are loaded at start', () => {
  render(<App />)
  expect(loadHosts).toHaveBeenCalledTimes(1)
})

test('the Hosts tab stays mounted and is hidden while another tab is active', () => {
  render(<App />)
  expect(screen.getByTestId('tabpane-hosts').className).toContain('hidden')
  act(() => {
    useSessionStore.getState().setActiveTab(null)
  })
  expect(screen.getByTestId('tabpane-hosts').className).not.toContain('hidden')
  expect(screen.getByTestId('hosts-tab')).toBeInTheDocument()
})

test('ctrl+shift+t opens the first local connection', () => {
  render(<App />)
  fireEvent.keyDown(window, { key: 'T', ctrlKey: true, shiftKey: true })
  const local = Object.values(useSessionStore.getState().panes).find((p) => p.kind === 'local')
  expect(local?.kind === 'local' && local.program).toBe('pwsh.exe')
})

function openDesk() {
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  const { rerender } = render(<App />)
  act(() => openConnection(deskHost, deskConn))
  return { tabs: useSessionStore.getState().tabs, rerender }
}

// A saved connection is dialled by id; the backend reads host, user and secret itself.
const deskTarget = { host: '', port: 0, username: null, secretId: null, profileId: 'vnc-1' }

test('opening a saved VNC connection opens a tab with only its id', () => {
  const { tabs } = openDesk()
  expect(tabs).toEqual([
    { id: tabs[0].id, kind: 'vnc', title: 'desk', connectionId: 'vnc-1', once: null },
  ])
  expect(tabSecretIds(tabs[0])).toEqual([])
  expect(vi.mocked(vncDriver).mock.calls).toEqual([[deskTarget]])
})

test('opening a saved VNC profile again opens another tab', () => {
  openDesk()
  act(() => openConnection(deskHost, deskConn))
  const { tabs } = useSessionStore.getState()
  expect(tabs).toHaveLength(2)
  expect(tabs[1]).toMatchObject({ kind: 'vnc', connectionId: 'vnc-1' })
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
    const replaced: Connection = {
      ...deskConn,
      kind: { type: 'vnc', port: 5901, username: 'faye', secretId: 'vault-replaced' },
    }
    useHostStore.setState({ hosts: [{ ...deskHost, connections: [replaced] }] })
  })
  rerender(<App />)
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
  expect(vi.mocked(vncDriver).mock.calls).toEqual([[deskTarget]])
})

function openOffice() {
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  const { rerender } = render(<App />)
  act(() => openConnection(officeHost, officeConn))
  return { tabs: useSessionStore.getState().tabs, rerender }
}

const officeTarget = {
  host: '',
  port: 0,
  username: '',
  domain: null,
  secretId: null,
  profileId: 'rdp-1',
  keyboardLayout: null,
}

test('opening a saved RDP connection opens a tab with only its id', () => {
  const { tabs } = openOffice()
  expect(tabs).toEqual([
    { id: tabs[0].id, kind: 'rdp', title: 'office', connectionId: 'rdp-1', once: null },
  ])
  expect(tabSecretIds(tabs[0])).toEqual([])
  expect(vi.mocked(rdpDriver).mock.calls).toEqual([[officeTarget]])
})

test('opening a saved RDP profile again opens another tab', () => {
  openOffice()
  act(() => openConnection(officeHost, officeConn))
  const { tabs } = useSessionStore.getState()
  expect(tabs).toHaveLength(2)
  expect(tabs[1]).toMatchObject({ kind: 'rdp', connectionId: 'rdp-1' })
  expect(tabs[1].id).not.toBe(tabs[0].id)
})

test('closing a tab of a saved RDP profile deletes no password', () => {
  const { tabs } = openOffice()
  act(() => {
    useSessionStore.getState().removeTab(tabs[0].id)
  })
  expect(useSessionStore.getState().tabs).toHaveLength(0)
  expect(deleteSecret).not.toHaveBeenCalled()
})

test('an RDP profile tab keeps its driver when the password or layout of the profile changes', () => {
  const { rerender } = openOffice()
  const { driver } = remote.props[0]
  act(() => {
    const changed: Connection = {
      ...officeConn,
      kind: {
        ...(officeConn.kind as Extract<Connection['kind'], { type: 'rdp' }>),
        keyboardLayout: 0x807,
      },
    }
    useHostStore.setState({ hosts: [{ ...officeHost, connections: [changed] }] })
  })
  rerender(<App />)
  expect(remote.props.length).toBeGreaterThan(1)
  for (const props of remote.props) expect(props.driver).toBe(driver)
  expect(vi.mocked(rdpDriver).mock.calls).toEqual([[officeTarget]])
})

test('the zoom keys zoom the active pane of the active tab and no other', () => {
  const local = (id: string) => ({
    id,
    kind: 'local' as const,
    title: 'Local shell',
    status: 'connecting' as const,
    reconnectNonce: 0,
    connectionId: null,
    program: null,
    startFolder: null,
  })
  const one = (id: string) => ({
    id: `t-${id}`,
    kind: 'panes' as const,
    paneIds: [id],
    direction: 'horizontal' as const,
    activePaneId: id,
  })
  useSessionStore.setState({
    tabs: [tab1, one('l1'), one('l2')],
    panes: { ...useSessionStore.getState().panes, l1: local('l1'), l2: local('l2') },
    activeTabId: 't-l1',
  })
  render(<App />)
  const zoom = () => ['s1', 'l1', 'l2'].map((id) => useSessionStore.getState().panes[id].zoom)
  fireEvent.keyDown(window, { key: '=', ctrlKey: true })
  fireEvent.keyDown(window, { key: '=', ctrlKey: true })
  expect(zoom()).toEqual([undefined, 2, undefined])
  fireEvent.keyDown(window, { key: '-', ctrlKey: true })
  expect(zoom()).toEqual([undefined, 1, undefined])
  fireEvent.keyDown(window, { key: '0', ctrlKey: true })
  expect(zoom()).toEqual([undefined, 0, undefined])
})
