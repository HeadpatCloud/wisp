import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  importSshConfig: vi.fn(),
  sshConfigApply: vi.fn(),
}))
vi.mock('@/bindings', () => ({ commands: m }))

import type { ImportCandidate } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { SshImportPage } from './SshImportPage'

const cand = (name: string, status: ImportCandidate['status']): ImportCandidate => ({
  name,
  host: `${name}.lan`,
  port: 22,
  username: 'me',
  keyPath: null,
  jumpHostAlias: null,
  status,
})

const candidates = [
  cand('web', { kind: 'new' }),
  cand('nas-ssh', { kind: 'addsTo', hostName: 'nas' }),
  cand('pi', { kind: 'alreadyThere', hostName: 'pi-hole' }),
]

beforeEach(() => {
  vi.clearAllMocks()
  m.importSshConfig.mockResolvedValue({ status: 'ok', data: candidates })
  m.sshConfigApply.mockResolvedValue({ status: 'ok', data: { hosts: 0, connections: 1 } })
  useHostStore.setState({ load: vi.fn().mockResolvedValue(undefined) })
})

test('the import page imports the ticked entries', async () => {
  const onClose = vi.fn()
  const user = userEvent.setup()
  render(<SshImportPage onClose={onClose} />)
  expect(await screen.findByText('adds to nas')).toBeInTheDocument()
  expect(screen.getByText('already in pi-hole')).toBeInTheDocument()
  expect(screen.getByRole('checkbox', { name: 'pi' })).toBeDisabled()
  await user.click(screen.getByRole('checkbox', { name: 'web' }))
  await user.click(screen.getByRole('button', { name: 'Import 1' }))
  // Already-there entries go along so a ProxyJump to them links; the backend doesn't import them.
  expect(m.sshConfigApply).toHaveBeenCalledWith([candidates[1], candidates[2]])
  expect(onClose).toHaveBeenCalled()
})
