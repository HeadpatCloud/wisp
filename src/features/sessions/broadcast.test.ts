import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/ssh', () => ({ writeSession: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@/lib/local', () => ({ writeLocal: vi.fn().mockResolvedValue(undefined) }))

import { writeLocal } from '@/lib/local'
import { writeSession } from '@/lib/ssh'
import { type PanesTab, useSessionStore } from '@/stores/sessionStore'
import { typeInto } from './broadcast'

const st = () => useSessionStore.getState()

beforeEach(() => {
  vi.clearAllMocks()
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
})

test('broadcast_types_into_every_terminal_pane', () => {
  st().openSsh('c1', 'web')
  const tab = () => st().tabs[0] as PanesTab
  const ssh = tab().paneIds[0]
  st().setSshId(ssh, 'ssh-1')
  st().setStatus(ssh, 'connected')
  st().openLocalShell(null, 'Local shell', null, null, { split: 'horizontal' })
  const local = tab().activePaneId
  st().setLocalId(local, 'local-1')
  st().openSftpProfile('f1', 'files', { split: 'horizontal' })

  expect(typeInto(local, 'ls\r')).toBe(false)
  expect(writeSession).not.toHaveBeenCalled()

  st().toggleBroadcast(tab().id)
  for (const from of [ssh, local]) {
    vi.clearAllMocks()
    expect(typeInto(from, 'ls\r')).toBe(true)
    expect(writeSession).toHaveBeenCalledWith('ssh-1', 'ls\r')
    expect(writeLocal).toHaveBeenCalledWith('local-1', 'ls\r')
  }
})
