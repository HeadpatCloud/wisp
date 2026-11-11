import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/tunnels', () => ({
  openBackground: vi.fn().mockResolvedValue('bg-1'),
  startTunnel: vi.fn().mockResolvedValue(undefined),
  stopTunnel: vi.fn().mockResolvedValue(undefined),
  forwardLabel: () => 'Local 127.0.0.1:8080 → db:5432',
}))

import type { Tunnel } from '@/bindings'
import { openBackground, startTunnel } from '@/lib/tunnels'
import { useNoticeStore } from '@/stores/noticeStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useTunnelStore } from '@/stores/tunnelStore'
import { runTunnel } from './run'

const tunnel = (id: string): Tunnel => ({
  id,
  kind: 'local',
  bindHost: '127.0.0.1',
  bindPort: 8080,
  targetHost: 'db',
  targetPort: 5432,
  autoStart: false,
})

beforeEach(() => {
  vi.clearAllMocks()
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
  useTunnelStore.setState({ byId: {}, background: {} })
})

test('start_uses_an_open_tab_or_a_background_connection', async () => {
  // No tab of the connection: one background connection, shared by its tunnels.
  await runTunnel('c1', tunnel('t1'))
  await runTunnel('c1', tunnel('t2'))
  expect(openBackground).toHaveBeenCalledTimes(1)
  expect(vi.mocked(startTunnel).mock.calls.map((c) => c[0])).toEqual(['bg-1', 'bg-1'])
  expect(useTunnelStore.getState().byId.t1).toMatchObject({ sessionId: 'bg-1', state: 'starting' })

  // A connected terminal of the connection carries it instead.
  useSessionStore.getState().openSsh('c2', 'web')
  const tab = useSessionStore.getState().tabs.at(-1)
  if (tab?.kind !== 'panes') throw new Error('no panes tab')
  useSessionStore.getState().setSshId(tab.paneIds[0], 'term-1')
  useSessionStore.getState().setStatus(tab.paneIds[0], 'connected')
  await runTunnel('c2', tunnel('t3'))
  expect(openBackground).toHaveBeenCalledTimes(1)
  expect(vi.mocked(startTunnel).mock.calls.at(-1)?.[0]).toBe('term-1')

  // A running tunnel isn't started again.
  await runTunnel('c2', tunnel('t3'))
  expect(startTunnel).toHaveBeenCalledTimes(3)
})

test('a failed start shows its reason', async () => {
  vi.mocked(startTunnel).mockRejectedValueOnce({
    kind: 'tunnel',
    message: '127.0.0.1:8080: in use',
  })
  await runTunnel('c1', tunnel('t1'))
  expect(useTunnelStore.getState().byId.t1).toMatchObject({
    state: 'error',
    message: '127.0.0.1:8080: in use',
  })
  expect(useNoticeStore.getState().notices.at(-1)).toMatchObject({ title: 'Tunnel stopped' })
})
