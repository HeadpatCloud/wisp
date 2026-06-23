import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

const v = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  vaultUnlock: vi.fn(),
  vaultChangePassword: vi.fn(),
}))
vi.mock('@/lib/vault', () => v)

import { VaultGate } from './VaultGate'

beforeEach(() => {
  vi.clearAllMocks()
  v.vaultUnlock.mockResolvedValue(undefined)
})

test('prompts for the master password when needed, then unlocks', async () => {
  v.vaultStatus.mockResolvedValue('needsPassword')
  const user = userEvent.setup()
  render(<VaultGate />)
  const input = await screen.findByLabelText(/master password/i)
  await user.type(input, 'hunter2')
  await user.click(screen.getByRole('button', { name: /unlock/i }))
  expect(v.vaultUnlock).toHaveBeenCalledWith('hunter2')
  await waitFor(() => expect(screen.queryByLabelText(/master password/i)).not.toBeInTheDocument())
})

test('stays out of the way when already unlocked', async () => {
  v.vaultStatus.mockResolvedValue('unlocked')
  render(<VaultGate />)
  await waitFor(() => expect(v.vaultStatus).toHaveBeenCalled())
  expect(screen.queryByLabelText(/master password/i)).not.toBeInTheDocument()
})

test('shows an error on wrong password and keeps prompting', async () => {
  v.vaultStatus.mockResolvedValue('needsPassword')
  v.vaultUnlock.mockRejectedValueOnce({ kind: 'wrongPassphrase' })
  const user = userEvent.setup()
  render(<VaultGate />)
  const input = await screen.findByLabelText(/master password/i)
  await user.type(input, 'bad')
  await user.click(screen.getByRole('button', { name: /unlock/i }))
  expect(await screen.findByText(/incorrect/i)).toBeInTheDocument()
  expect(screen.getByLabelText(/master password/i)).toBeInTheDocument()
})
