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

test('host key prompts keep their wording', () => {
  const { rerender } = render(
    <HostKeyDialog
      prompt={{ kind: 'unknown', host: 'h', port: 22, fingerprint: 'SHA256:abc' }}
      onAccept={vi.fn()}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByRole('heading', { name: 'Unknown host key' })).toBeInTheDocument()
  expect(screen.getByText('h:22')).toBeInTheDocument()
  expect(screen.getByText('First connection to this host. Trust this key?')).toBeInTheDocument()

  rerender(
    <HostKeyDialog
      prompt={{ kind: 'mismatch', host: 'h', port: 22, stored: 'old', offered: 'new' }}
      onAccept={vi.fn()}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByRole('heading', { name: 'Host key CHANGED' })).toBeInTheDocument()
  expect(
    screen.getByText(
      'The host key has changed - this may indicate a man-in-the-middle attack. Only accept if you know the key was rotated.',
    ),
  ).toBeInTheDocument()
})

test('unknown certificate prompt hides the scheme prefix', () => {
  render(
    <HostKeyDialog
      prompt={{
        kind: 'unknown',
        host: 'vnc/desk',
        port: 5900,
        fingerprint: 'SHA256:abc',
        certificate: true,
      }}
      onAccept={vi.fn()}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByRole('heading', { name: 'Unknown certificate' })).toBeInTheDocument()
  expect(screen.getByText('desk:5900')).toBeInTheDocument()
  expect(
    screen.getByText('First connection to this server. Trust this certificate?'),
  ).toBeInTheDocument()
  expect(screen.getByText('SHA256:abc')).toBeInTheDocument()
})

test('changed certificate prompt warns and shows stored vs offered', () => {
  render(
    <HostKeyDialog
      prompt={{
        kind: 'mismatch',
        host: 'rdp/desk',
        port: 3389,
        stored: 'old',
        offered: 'new',
        certificate: true,
      }}
      onAccept={vi.fn()}
      onReject={vi.fn()}
    />,
  )
  expect(screen.getByRole('heading', { name: 'Certificate CHANGED' })).toBeInTheDocument()
  expect(screen.getByText('desk:3389')).toBeInTheDocument()
  expect(
    screen.getByText(
      'The certificate has changed - this may indicate a man-in-the-middle attack. Only accept if you know it was replaced.',
    ),
  ).toBeInTheDocument()
  expect(screen.getByText(/stored: old/)).toBeInTheDocument()
  expect(screen.getByText(/offered: new/)).toBeInTheDocument()
})
