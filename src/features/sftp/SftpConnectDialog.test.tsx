import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
import { SftpConnectDialog } from './SftpConnectDialog'

const profile = (id: string, name: string, host: string) =>
  ({
    id,
    name,
    host,
    groupId: null,
    port: 22,
    username: 'u',
    authMethod: 'agent',
    keyPath: null,
    secretId: null,
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
    jumpHostId: null,
    tunnels: [],
  }) as never

beforeEach(() => {
  useProfileStore.setState({
    profiles: [profile('p1', 'web-01', '10.0.0.1'), profile('p2', 'db-02', '10.0.0.2')],
    groups: [],
    loaded: true,
  } as never)
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
