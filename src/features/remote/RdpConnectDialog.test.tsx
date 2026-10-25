import { fireEvent, render, screen } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, expect, test, vi } from 'vitest'
import { RdpConnectDialog } from './RdpConnectDialog'

const DOMAIN_RULE = 'Put the domain either in the user name or in the Domain field.'

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
      <RdpConnectDialog open={open} onOpenChange={setOpen} onConnect={onConnect} />
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

test('Connect passes the host, port, username, domain and password', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), 'pc.example.com')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '3390' } })
  await user.type(screen.getByLabelText('Username'), 'alice')
  await user.type(screen.getByLabelText('Domain (optional)'), 'CORP')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('pc.example.com', 3390, 'alice', 'CORP', 'hunter2')
})

test('the default port is 3389 and an empty domain becomes null', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  expect(screen.getByLabelText('Port')).toHaveValue(3389)
  await user.type(screen.getByLabelText('Host'), 'pc')
  await user.type(screen.getByLabelText('Username'), 'alice')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('pc', 3389, 'alice', null, '')
})

test('spaces around the host, username and domain are dropped', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), ' pc ')
  await user.type(screen.getByLabelText('Username'), '  alice ')
  await user.type(screen.getByLabelText('Domain (optional)'), ' CORP  ')
  await user.type(screen.getByLabelText('Password'), ' hunter2 ')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('pc', 3389, 'alice', 'CORP', ' hunter2 ')
})

test.each(['', '   '])('a domain of "%s" is passed as null', async (domain) => {
  const onConnect = vi.fn()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'pc' } })
  fireEvent.change(screen.getByLabelText('Domain (optional)'), { target: { value: domain } })
  fireEvent.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenCalledWith('pc', 3389, '', null, '')
})

test.each(['', '   '])('Connect is disabled for a host of "%s"', (host) => {
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={vi.fn()} />)
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: host } })
  expect(screen.getByRole('button', { name: 'Connect' })).toBeDisabled()
})

test('Connect needs a port from 1 to 65535', () => {
  const onConnect = vi.fn()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  const connect = screen.getByRole('button', { name: 'Connect' })
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'pc' } })
  expect(connect).toBeEnabled()
  for (const port of ['', '0', '65536', '70000', '-1', '3389.5']) {
    fireEvent.change(screen.getByLabelText('Port'), { target: { value: port } })
    expect(connect, port).toBeDisabled()
  }
  for (const port of ['1', '65535']) {
    fireEvent.change(screen.getByLabelText('Port'), { target: { value: port } })
    expect(connect, port).toBeEnabled()
  }
  fireEvent.click(connect)
  expect(onConnect).toHaveBeenCalledWith('pc', 65535, '', null, '')
})

test.each(['CORP\\alice', 'alice@corp.example', ' CORP\\alice '])(
  'a domain together with "%s" as the user name is refused',
  async (username) => {
    const onConnect = vi.fn()
    render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
    expect(screen.queryByText(DOMAIN_RULE)).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'pc' } })
    fireEvent.change(screen.getByLabelText('Username'), { target: { value: username } })
    fireEvent.change(screen.getByLabelText('Domain (optional)'), { target: { value: 'CORP' } })

    expect(screen.getByText(DOMAIN_RULE)).toBeInTheDocument()
    const connect = screen.getByRole('button', { name: 'Connect' })
    expect(connect).toBeDisabled()
    fireEvent.click(connect)
    expect(onConnect).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('Domain (optional)'), { target: { value: '  ' } })
    expect(screen.queryByText(DOMAIN_RULE)).not.toBeInTheDocument()
    expect(connect).toBeEnabled()
    fireEvent.click(connect)
    expect(onConnect).toHaveBeenCalledWith('pc', 3389, username.trim(), null, '')
  },
)

test('the domain rule describes the Domain field and marks it invalid', () => {
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={vi.fn()} />)
  const domain = screen.getByLabelText('Domain (optional)')
  fireEvent.change(screen.getByLabelText('Username'), { target: { value: 'alice@corp.example' } })
  fireEvent.change(domain, { target: { value: 'CORP' } })
  expect(domain).toHaveAccessibleDescription(DOMAIN_RULE)
  expect(domain).toHaveAttribute('aria-invalid', 'true')

  fireEvent.change(domain, { target: { value: '' } })
  expect(domain).not.toHaveAccessibleDescription(DOMAIN_RULE)
  expect(domain).not.toHaveAttribute('aria-invalid', 'true')
})

test('a domain with a plain user name is fine', () => {
  const onConnect = vi.fn()
  render(<RdpConnectDialog open onOpenChange={vi.fn()} onConnect={onConnect} />)
  fireEvent.change(screen.getByLabelText('Host'), { target: { value: 'pc' } })
  fireEvent.change(screen.getByLabelText('Username'), { target: { value: 'alice' } })
  fireEvent.change(screen.getByLabelText('Domain (optional)'), { target: { value: 'CORP' } })

  expect(screen.queryByText(DOMAIN_RULE)).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Connect' })).toBeEnabled()
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
  await user.type(screen.getByLabelText('Host'), 'pc')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '3390' } })
  await user.type(screen.getByLabelText('Username'), 'alice')
  await user.type(screen.getByLabelText('Domain (optional)'), 'CORP')
  await user.type(screen.getByLabelText('Password'), 'hunter2')

  await close(user)
  expect(screen.queryByLabelText('Password')).not.toBeInTheDocument()

  await user.click(screen.getByRole('button', { name: 'Reopen' }))
  expect(screen.getByLabelText('Host')).toHaveValue('')
  expect(screen.getByLabelText('Port')).toHaveValue(3389)
  expect(screen.getByLabelText('Username')).toHaveValue('')
  expect(screen.getByLabelText('Domain (optional)')).toHaveValue('')
  expect(screen.getByLabelText('Password')).toHaveValue('')

  await user.type(screen.getByLabelText('Host'), 'laptop')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(onConnect).toHaveBeenLastCalledWith('laptop', 3389, '', null, '')
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
  await user.type(screen.getByLabelText('Host'), 'pc')
  fireEvent.change(screen.getByLabelText('Port'), { target: { value: '3390' } })
  await user.type(screen.getByLabelText('Username'), 'alice')
  await user.type(screen.getByLabelText('Domain (optional)'), 'CORP')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Connect' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(screen.getByLabelText('Host')).toHaveValue('pc')
  expect(screen.getByLabelText('Port')).toHaveValue(3390)
  expect(screen.getByLabelText('Username')).toHaveValue('alice')
  expect(screen.getByLabelText('Domain (optional)')).toHaveValue('CORP')
  expect(screen.getByLabelText('Password')).toHaveValue('')

  fireEvent.click(screen.getByRole('button', { name: 'Reopen', hidden: true }))
  expect(screen.getByLabelText('Host')).toHaveValue('')
  expect(screen.getByLabelText('Port')).toHaveValue(3389)
  expect(screen.getByLabelText('Username')).toHaveValue('')
  expect(screen.getByLabelText('Domain (optional)')).toHaveValue('')
  expect(screen.getByLabelText('Password')).toHaveValue('')
})

test('a second click on Connect while the dialog closes connects nothing more', async () => {
  keepClosingDialog()
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<Reopenable onConnect={onConnect} />)
  await user.type(screen.getByLabelText('Host'), 'pc')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  const connect = screen.getByRole('button', { name: 'Connect' })
  await user.click(connect)
  const content = document.querySelector('[data-slot="dialog-content"]')
  expect(content).toHaveAttribute('data-state', 'closed')
  expect(connect).toBeDisabled()
  expect(content).not.toContainElement(document.activeElement as HTMLElement)

  fireEvent.click(connect)
  expect(onConnect.mock.calls).toEqual([['pc', 3389, '', null, 'hunter2']])
})
