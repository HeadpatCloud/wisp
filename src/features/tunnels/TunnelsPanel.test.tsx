import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  startTunnel: vi.fn().mockResolvedValue(undefined),
  stopTunnel: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/tunnels', () => ({ startTunnel: m.startTunnel, stopTunnel: m.stopTunnel }))

import { useTunnelStore } from '@/stores/tunnelStore'
import { TunnelsPanel } from './TunnelsPanel'

const tunnels = [
  {
    id: 't1',
    kind: 'local',
    bindHost: '127.0.0.1',
    bindPort: 8080,
    targetHost: 'db',
    targetPort: 5432,
    autoStart: false,
  },
] as never[]

beforeEach(() => {
  vi.clearAllMocks()
  useTunnelStore.setState({ byId: {} })
})

test('start button calls startTunnel with the live sshId + tunnel', async () => {
  const user = userEvent.setup()
  render(<TunnelsPanel sessionId="ssh-1" profileTunnels={tunnels} />)
  expect(screen.getByText(/8080/)).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: /start/i }))
  expect(m.startTunnel).toHaveBeenCalledWith('ssh-1', expect.objectContaining({ id: 't1' }))
})

test('shows stop + counters when active', async () => {
  useTunnelStore.setState({
    byId: {
      t1: { tunnelId: 't1', sessionId: 'ssh-1', state: 'active', bytesUp: 100, bytesDown: 50 },
    },
  })
  render(<TunnelsPanel sessionId="ssh-1" profileTunnels={tunnels} />)
  expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument()
})
