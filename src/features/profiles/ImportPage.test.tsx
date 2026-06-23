import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const saveProfile = vi.fn().mockResolvedValue(undefined)
const removeTab = vi.fn()
vi.mock('@/bindings', () => ({}))
vi.mock('@/lib/profiles', () => ({ importSshConfig: vi.fn(), nextProfileOrder: () => 0 }))

import { importSshConfig } from '@/lib/profiles'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { ImportPage } from './ImportPage'

const candidates = [
  {
    name: 'web',
    host: '1.2.3.4',
    port: 22,
    username: 'me',
    keyPath: '/k',
    jumpHostAlias: 'bastion',
  },
  { name: 'bastion', host: 'b', port: 22, username: 'me', keyPath: null, jumpHostAlias: null },
] as never[]

beforeEach(() => {
  vi.clearAllMocks()
  useProfileStore.setState({ groups: [], profiles: [], loaded: true, saveProfile } as never)
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null, removeTab } as never)
})

test('imports selected candidates and links jump host by name', async () => {
  vi.mocked(importSshConfig).mockResolvedValue(candidates)
  const user = userEvent.setup()
  render(<ImportPage tabId="i1" />)
  expect(await screen.findByText(/web - me@1.2.3.4:22/)).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Import' }))
  expect(saveProfile).toHaveBeenCalledTimes(2)
  const webCall = saveProfile.mock.calls.find((c) => c[0].name === 'web')?.[0]
  const bastionCall = saveProfile.mock.calls.find((c) => c[0].name === 'bastion')?.[0]
  expect(webCall.jumpHostId).toBe(bastionCall.id)
  expect(webCall.authMethod).toBe('key')
  expect(bastionCall.authMethod).toBe('password')
  expect(removeTab).toHaveBeenCalledWith('i1')
})

test('shows error message when importSshConfig rejects', async () => {
  vi.mocked(importSshConfig).mockRejectedValue(new Error('read failed'))
  render(<ImportPage tabId="i1" />)
  expect(await screen.findByText('Could not read the SSH config file.')).toBeInTheDocument()
})
