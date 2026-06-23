import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const ssh = vi.hoisted(() => ({
  connectSession: vi.fn().mockResolvedValue('rust-sid'),
  disconnectSession: vi.fn().mockResolvedValue(undefined),
  writeSession: vi.fn(),
  resizeSession: vi.fn(),
  trustHostKey: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/ssh', () => ssh)
vi.mock('@/lib/tunnels', () => ({ startTunnel: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@/bindings', () => ({
  events: {
    sshStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
    tunnelStatus: { listen: vi.fn().mockResolvedValue(() => undefined) },
  },
}))
vi.mock('./TerminalView', async () => {
  const { useEffect } = await import('react')
  return {
    TerminalView: ({ onResize }: { onResize: (cols: number, rows: number) => void }) => {
      useEffect(() => onResize(120, 40), [onResize])
      return <div data-testid="terminal" />
    },
  }
})
vi.mock('../sftp/SftpPanel', () => ({ SftpPanel: () => <div data-testid="sftp" /> }))
vi.mock('../tunnels/TunnelsPanel', () => ({ TunnelsPanel: () => <div data-testid="tunnels" /> }))

import type { SessionTab } from '@/stores/sessionStore'
import { useSessionStore } from '@/stores/sessionStore'
import { PaneView } from './PaneView'

beforeEach(() => {
  vi.clearAllMocks()
  useSessionStore.setState({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
    ],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'web-01', status: 'connected', reconnectNonce: 0 },
    },
    activeTabId: 't1',
  })
})

test('connects using the terminal fitted size, not a hardcoded default', async () => {
  render(<PaneView tabId="t1" sessionId="s1" />)
  await waitFor(() => expect(ssh.connectSession).toHaveBeenCalled())
  expect(ssh.connectSession).toHaveBeenCalledWith('p1', 120, 40, expect.any(Function))
})

test('renders the terminal and splits right', async () => {
  const user = userEvent.setup()
  render(<PaneView tabId="t1" sessionId="s1" />)
  expect(screen.getByTestId('terminal')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: /split right/i }))
  expect((useSessionStore.getState().tabs[0] as SessionTab).sessionIds).toHaveLength(2)
  expect((useSessionStore.getState().tabs[0] as SessionTab).direction).toBe('horizontal')
})

test('split down sets vertical direction', async () => {
  const user = userEvent.setup()
  render(<PaneView tabId="t1" sessionId="s1" />)
  await user.click(screen.getByRole('button', { name: /split down/i }))
  expect((useSessionStore.getState().tabs[0] as SessionTab).direction).toBe('vertical')
})

test('close pane removes the now-empty tab', async () => {
  const user = userEvent.setup()
  render(<PaneView tabId="t1" sessionId="s1" />)
  await user.click(screen.getByRole('button', { name: /close pane/i }))
  expect(useSessionStore.getState().tabs).toHaveLength(0)
})

test('host-key-unknown error opens the dialog; Trust records and reconnects', async () => {
  const user = userEvent.setup()
  ssh.connectSession.mockRejectedValueOnce({
    kind: 'hostKeyUnknown',
    message: { host: 'h', port: 22, fingerprint: 'SHA256:zzz' },
  })
  useSessionStore.setState({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
    ],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'web-01', status: 'connecting', reconnectNonce: 0 },
    },
    activeTabId: 't1',
  })
  render(<PaneView tabId="t1" sessionId="s1" />)
  expect(await screen.findByText('SHA256:zzz')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: /trust/i }))
  expect(ssh.trustHostKey).toHaveBeenCalledWith('h', 22, 'SHA256:zzz')
  expect(useSessionStore.getState().sessions.s1.reconnectNonce).toBe(1)
})

test('wrong-passphrase error surfaces an actionable message', async () => {
  ssh.connectSession.mockRejectedValueOnce({ kind: 'wrongPassphrase' })
  useSessionStore.setState({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
    ],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'web-01', status: 'connecting', reconnectNonce: 0 },
    },
    activeTabId: 't1',
  })
  render(<PaneView tabId="t1" sessionId="s1" />)
  expect(await screen.findByText(/passphrase/i)).toBeInTheDocument()
  expect(useSessionStore.getState().sessions.s1.status).toBe('error')
})

test('shows Reconnect when closed and reconnect bumps the session', async () => {
  ssh.connectSession.mockReturnValue(new Promise(() => undefined))
  const user = userEvent.setup()
  useSessionStore.setState({
    tabs: [
      {
        id: 't1',
        kind: 'session',
        sessionIds: ['s1'],
        direction: 'horizontal',
        activePaneId: 's1',
      },
    ],
    sessions: {
      s1: { id: 's1', profileId: 'p1', title: 'web-01', status: 'closed', reconnectNonce: 0 },
    },
    activeTabId: 't1',
  })
  render(<PaneView tabId="t1" sessionId="s1" />)
  await user.click(screen.getByRole('button', { name: /reconnect/i }))
  expect(useSessionStore.getState().sessions.s1.status).toBe('connecting')
  expect(useSessionStore.getState().sessions.s1.reconnectNonce).toBe(1)
})
