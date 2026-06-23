import { beforeEach, expect, test } from 'vitest'
import { useTunnelStore } from './tunnelStore'

const rt = (id: string, sid = 's1') => ({
  tunnelId: id,
  sessionId: sid,
  state: 'starting' as const,
  bytesUp: 0,
  bytesDown: 0,
})

beforeEach(() => useTunnelStore.setState({ byId: {} }))

test('start adds runtime; setStatus updates it', () => {
  useTunnelStore.getState().start(rt('t1'))
  useTunnelStore
    .getState()
    .setStatus({ tunnelId: 't1', state: 'active', bytesUp: 10, bytesDown: 5 })
  expect(useTunnelStore.getState().byId.t1.state).toBe('active')
  expect(useTunnelStore.getState().byId.t1.bytesUp).toBe(10)
})

test('setStatus ignores unknown tunnels', () => {
  useTunnelStore
    .getState()
    .setStatus({ tunnelId: 'ghost', state: 'active', bytesUp: 1, bytesDown: 1 })
  expect(useTunnelStore.getState().byId.ghost).toBeUndefined()
})

test('clearSession drops all tunnels for a session', () => {
  const s = useTunnelStore.getState()
  s.start(rt('t1', 's1'))
  s.start(rt('t2', 's2'))
  s.clearSession('s1')
  expect(Object.keys(useTunnelStore.getState().byId)).toEqual(['t2'])
})
