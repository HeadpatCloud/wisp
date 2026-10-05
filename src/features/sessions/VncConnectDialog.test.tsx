import { fireEvent, render, screen } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, expect, test, vi } from 'vitest'
import { VncConnectDialog } from './VncConnectDialog'

afterEach(() => {
  vi.unstubAllGlobals()
})

function Reopenable({ onConnect }: { onConnect: () => void }) {
  const [open, setOpen] = useState(true)
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Reopen
      </button>
      <VncConnectDialog open={open} onOpenChange={setOpen} onConnect={onConnect} />
    </>
  )
}

// An exit animation keeps the content mounted after the dialog has closed.
function keepClosingDialog() {
  const computed = window.getComputedStyle.bind(window)
  vi.stubGlobal('getComputedStyle', (el: Element, pseudo?: string) => {
    const styles = computed(el, pseudo)
    if (!el.getAttribute('data-slot')?.startsWith('dialog-')) return styles
    return new Proxy(styles, {
      get(target, prop) {
        if (prop === 'animationName') {
          return el.getAttribute('data-state') === 'open' ? 'enter' : 'exit'
        }
        const value = Reflect.get(target, prop)
        return typeof value === 'function' ? value.bind(target) : value
      },
    })
  })
}

test('Connect passes the host, port, username and password', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<VncConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), 'mac.example.com')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '5901' } })
  await user.type(screen.getByLabelText('Username (optional)'), 'alice')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('mac.example.com', 5901, 'alice', 'hunter2')
})

test('the username can be left empty and says when it is needed', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<VncConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  expect(
    screen.getByText('Only for macOS Screen Sharing and servers that ask for one.'),
  ).toBeInTheDocument()
  await user.type(screen.getByLabelText('Host'), 'mac.example.com')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('mac.example.com', 5900, '', '')
})

test('Connect is disabled until a host is entered', () => {
  render(<VncConnectDialog open onOpenChange={vi.fn()} onConnect={vi.fn()} />)
  expect(screen.getByRole('button', { name: 'Connect' })).toBeDisabled()
})

test('Connect needs a port from 1 to 65535', () => {
  const onConnect = vi.fn()
  render(<VncConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  const connect = screen.getByRole('button', { name: 'Connect' })
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'mac' } })
  expect(connect).toBeEnabled()
  for (const port of ['', '0', '65536', '70000', '-1', '5900.5']) {
    fireEvent.change(screen.getByLabelText('Port'), { target: { value: port } })
    expect(connect, port).toBeDisabled()
  }
  for (const port of ['1', '65535']) {
    fireEvent.change(screen.getByLabelText('Port'), { target: { value: port } })
    expect(connect, port).toBeEnabled()
  }
  fireEvent.click(connect)
  expect(onConnect).toHaveBeenCalledWith('mac', 65535, '', '')
})

test.each<[string, (user: UserEvent) => Promise<void>]>([
  ['Connect', (user) => user.click(screen.getByRole('button', { name: 'Connect' }))],
  ['Cancel', (user) => user.click(screen.getByRole('button', { name: 'Cancel' }))],
  ['Escape', (user) => user.keyboard('{Escape}')],
  ['the close button', (user) => user.click(screen.getByRole('button', { name: 'Close' }))],
  [
    'a click outside',
    async (user) => {
      const overlay = document.querySelector('[data-slot="dialog-overlay"]')
      if (!overlay) throw new Error('expected the dialog overlay')
      await user.click(overlay)
    },
  ],
])('closing with %s forgets what was typed', async (_how, close) => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<Reopenable onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), 'mac')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '5901' } })
  await user.type(screen.getByLabelText('Username (optional)'), 'alice')
  await user.type(screen.getByLabelText('Password'), 'hunter2')

  await close(user)
  expect(screen.queryByLabelText('Password')).not.toBeInTheDocument()

  await user.click(screen.getByRole('button', { name: 'Reopen' }))
  expect(screen.getByLabelText('Host')).toHaveValue('')
  expect(screen.getByLabelText('Port')).toHaveValue(5900)
  expect(screen.getByLabelText('Username (optional)')).toHaveValue('')
  expect(screen.getByLabelText('Password')).toHaveValue('')

  await user.type(screen.getByLabelText('Host'), 'pc')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenLastCalledWith('pc', 5900, '', '')
})

test('the password is gone while the closed dialog is still fading out', async () => {
  keepClosingDialog()
  const user = userEvent.setup()
  render(<Reopenable onConnect={vi.fn()} />)
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(screen.getByLabelText('Password')).toHaveValue('')
})

test('the other fields stay as typed while the closed dialog is still fading out', async () => {
  keepClosingDialog()
  const user = userEvent.setup()
  render(<Reopenable onConnect={vi.fn()} />)
  await user.type(screen.getByLabelText('Host'), 'mac')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '5901' } })
  await user.type(screen.getByLabelText('Username (optional)'), 'alice')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(screen.getByLabelText('Host')).toHaveValue('mac')
  expect(screen.getByLabelText('Port')).toHaveValue(5901)
  expect(screen.getByLabelText('Username (optional)')).toHaveValue('alice')
  expect(screen.getByLabelText('Password')).toHaveValue('')

  fireEvent.click(screen.getByRole('button', { name: 'Reopen', hidden: true }))
  expect(screen.getByLabelText('Host')).toHaveValue('')
  expect(screen.getByLabelText('Port')).toHaveValue(5900)
  expect(screen.getByLabelText('Username (optional)')).toHaveValue('')
  expect(screen.getByLabelText('Password')).toHaveValue('')
})
