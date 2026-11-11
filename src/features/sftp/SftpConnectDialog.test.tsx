import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import type { Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { SftpConnectDialog } from './SftpConnectDialog'

const host = (id: string, name: string, address: string): Host => ({
  id: `h-${id}`,
  name,
  address,
  folderId: null,
  color: null,
  order: 0,
  connections: [
    {
      id,
      label: null,
      address: null,
      kind: {
        type: 'ssh',
        port: 22,
        login: { kind: 'own', username: 'u', authMethod: 'agent', keyIds: [], secretId: null },
        jumpHostId: null,
        tunnels: [],
        appearance: null,
      },
    },
  ],
})

beforeEach(() => {
  useHostStore.setState({
    hosts: [host('p1', 'web-01', '10.0.0.1'), host('p2', 'db-02', '10.0.0.2')],
    folders: [],
    logins: [],
    keys: [],
    tags: [],
    loaded: true,
  })
})

test('filters hosts by query and picks one', async () => {
  const onPick = vi.fn()
  const user = userEvent.setup()
  render(<SftpConnectDialog open onOpenChange={vi.fn()} onPick={onPick} onConnect={vi.fn()} />)
  expect(screen.getByText('web-01')).toBeInTheDocument()
  await user.type(screen.getByPlaceholderText('Search hosts...'), 'db')
  expect(screen.queryByText('web-01')).not.toBeInTheDocument()
  await user.click(screen.getByText('db-02'))
  expect(onPick).toHaveBeenCalledWith('p2', 'db-02')
})

test('reopens on the picker with a cleared manual form', async () => {
  const user = userEvent.setup()
  const { rerender } = render(
    <SftpConnectDialog open onOpenChange={vi.fn()} onPick={vi.fn()} onConnect={vi.fn()} />,
  )
  await user.click(screen.getByText('Connect manually…'))
  await user.type(screen.getByLabelText('Host'), 'example.com')
  rerender(
    <SftpConnectDialog open={false} onOpenChange={vi.fn()} onPick={vi.fn()} onConnect={vi.fn()} />,
  )
  rerender(<SftpConnectDialog open onOpenChange={vi.fn()} onPick={vi.fn()} onConnect={vi.fn()} />)
  expect(screen.getByPlaceholderText('Search hosts...')).toBeInTheDocument()
  await user.click(screen.getByText('Connect manually…'))
  expect(screen.getByLabelText('Host')).toHaveValue('')
})

test('connects manually without a saved profile', async () => {
  const onConnect = vi.fn()
  const user = userEvent.setup()
  render(<SftpConnectDialog open onOpenChange={vi.fn()} onPick={vi.fn()} onConnect={onConnect} />)
  await user.click(screen.getByText('Connect manually…'))
  const connect = screen.getByRole('button', { name: 'Connect' })
  expect(connect).toBeDisabled()
  await user.type(screen.getByLabelText('Host'), 'example.com')
  await user.type(screen.getByLabelText('Username'), 'root')
  await user.type(screen.getByLabelText('Password'), 'hunter2')
  await user.click(connect)
  expect(onConnect).toHaveBeenCalledWith({
    host: 'example.com',
    port: 22,
    username: 'root',
    authMethod: 'password',
    keyPath: '',
    secret: 'hunter2',
  })
})
