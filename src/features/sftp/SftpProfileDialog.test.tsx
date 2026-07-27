import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn().mockResolvedValue('/keys/picked'),
}))
// The id must not embed the secret, or the "never stores plaintext" assertion is vacuous.
vi.mock('@/lib/vault', () => ({
  setSecret: vi.fn().mockResolvedValue('vault-1'),
  deleteSecret: vi.fn().mockResolvedValue(undefined),
}))

import { setSecret } from '@/lib/vault'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import { SftpProfileDialog } from './SftpProfileDialog'

const save = vi.fn().mockResolvedValue(undefined)

beforeEach(() => {
  vi.clearAllMocks()
  useSftpProfileStore.setState({ profiles: [], loaded: true, save } as never)
})

test('saves a password profile with the secret parked in the vault', async () => {
  const user = userEvent.setup()
  render(<SftpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.type(screen.getByLabelText('Host'), 'files.example.com')
  await user.type(screen.getByLabelText('Username'), 'deploy')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Save' }))

  await waitFor(() => expect(save).toHaveBeenCalled())
  const saved = save.mock.calls.at(-1)?.[0]
  expect(saved.host).toBe('files.example.com')
  expect(saved.name).toBe('files.example.com')
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(saved.secretId).toBe('vault-1')
  // The profile carries the reference, never the plaintext.
  expect(JSON.stringify(saved)).not.toContain('hunter2')
})

test('saves several keys in the order shown', async () => {
  const user = userEvent.setup()
  render(<SftpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.type(screen.getByLabelText('Host'), 'h')
  await user.type(screen.getByLabelText('Username'), 'u')
  await user.click(screen.getByLabelText('Auth method'))
  await user.click((await screen.findAllByText('Private key')).at(-1) as HTMLElement)

  await user.click(await screen.findByRole('button', { name: /add key/i }))
  await user.type(screen.getByPlaceholderText('/home/me/.ssh/id_ed25519'), '/keys/a')
  await user.click(screen.getByRole('button', { name: /add key/i }))
  await user.type(screen.getAllByPlaceholderText('/home/me/.ssh/id_ed25519')[1], '/keys/b')
  await user.click(screen.getByRole('button', { name: 'Save' }))

  await waitFor(() => expect(save).toHaveBeenCalled())
  const saved = save.mock.calls.at(-1)?.[0]
  expect(saved.keys.map((k: { path: string }) => k.path)).toEqual(['/keys/a', '/keys/b'])
})
