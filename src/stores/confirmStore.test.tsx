import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, test } from 'vitest'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { confirm } from './confirmStore'

test('confirm resolves by the button pressed', async () => {
  const user = userEvent.setup()
  render(<ConfirmDialog />)
  const ask = () =>
    confirm({
      title: 'Delete nas.local?',
      body: 'Its 3 connections go with it.',
      confirmLabel: 'Delete',
      destructive: true,
    })
  const yes = ask()
  await user.click(await screen.findByRole('button', { name: 'Delete' }))
  await expect(yes).resolves.toBe(true)
  const no = ask()
  await user.click(await screen.findByRole('button', { name: 'Cancel' }))
  await expect(no).resolves.toBe(false)
})
