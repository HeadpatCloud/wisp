import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, test, vi } from 'vitest'
import { HostKeyDialog } from './HostKeyDialog'

test('unknown prompt shows fingerprint and Trust calls onAccept', async () => {
  const onAccept = vi.fn()
  const user = userEvent.setup()
  render(
    <HostKeyDialog
      prompt={{ kind: 'unknown', host: 'h', port: 22, fingerprint: 'SHA256:abc' }}
      onAccept={onAccept}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByText('SHA256:abc')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: /trust/i }))
  expect(onAccept).toHaveBeenCalled()
})

test('mismatch prompt warns and shows stored vs offered', () => {
  render(
    <HostKeyDialog
      prompt={{ kind: 'mismatch', host: 'h', port: 22, stored: 'old', offered: 'new' }}
      onAccept={vi.fn()}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByText(/man-in-the-middle/i)).toBeInTheDocument()
  expect(screen.getByText(/stored: old/)).toBeInTheDocument()
  expect(screen.getByText(/offered: new/)).toBeInTheDocument()
})
