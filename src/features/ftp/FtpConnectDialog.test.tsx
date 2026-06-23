import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, test, vi } from 'vitest'
import { FtpConnectDialog } from './FtpConnectDialog'

test('Connect submits the entered params with FTP defaults', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<FtpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), 'ftp.example.com')
  await user.type(screen.getByLabelText('Username'), 'bob')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith(
    expect.objectContaining({ host: 'ftp.example.com', username: 'bob', port: 21, secure: false }),
  )
})

test('Connect is disabled until a host is entered', () => {
  render(<FtpConnectDialog open onOpenChange={vi.fn()} onConnect={vi.fn()} />)
  expect(screen.getByRole('button', { name: 'Connect' })).toBeDisabled()
})
