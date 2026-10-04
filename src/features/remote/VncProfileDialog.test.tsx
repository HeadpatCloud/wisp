import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { VncProfile } from '@/bindings'

vi.mock('@/lib/vault', () => ({
  setSecret: vi.fn(),
  deleteSecret: vi.fn(),
}))

import { deleteSecret, setSecret } from '@/lib/vault'
import { useVncProfileStore } from '@/stores/vncProfileStore'
import { VncProfileDialog } from './VncProfileDialog'

const desk: VncProfile = {
  id: 'vnc-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 5901,
  username: 'faye',
  secretId: 'vault-old',
  icon: { kind: 'builtin', name: 'cloud' },
  order: 4,
}

const save = vi.fn()
// Every vault and store call in the order it was made. The new id never embeds the password.
let calls: string[] = []

beforeEach(() => {
  vi.resetAllMocks()
  calls = []
  vi.mocked(setSecret).mockImplementation(async () => {
    calls.push('setSecret')
    return 'vault-new'
  })
  vi.mocked(deleteSecret).mockImplementation(async (id) => {
    calls.push(`deleteSecret ${id}`)
  })
  save.mockImplementation(async (p: VncProfile) => {
    calls.push(`save ${p.secretId}`)
  })
  useVncProfileStore.setState({ profiles: [], loaded: true, save } as never)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

const field = (label: string) => screen.getByLabelText(label)
const saveButton = () => screen.getByRole('button', { name: 'Save' })
const saved = () => save.mock.calls.at(-1)?.[0] as VncProfile

function Reopenable({ editing }: { editing: VncProfile | null }) {
  const [open, setOpen] = useState(true)
  const [target, setTarget] = useState(editing)
  const reopen = (next: VncProfile | null) => {
    setTarget(next)
    setOpen(true)
  }
  return (
    <>
      <button type="button" onClick={() => reopen(null)}>
        New profile
      </button>
      <button type="button" onClick={() => reopen(desk)}>
        Edit desk
      </button>
      <VncProfileDialog open={open} onOpenChange={setOpen} editing={target} />
    </>
  )
}

test('creating stores the password in the vault and saves the profile', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  expect(screen.getByText('New VNC profile')).toBeInTheDocument()
  expect(field('Port')).toHaveValue(5900)
  await user.type(field('Name'), 'Office Mac')
  await user.type(field('Host'), '  mac.example.com ')
  fireEvent.change(field('Port'), { target: { value: '5901' } })
  await user.type(field('Username (optional)'), ' alice ')
  await user.type(field('Password'), 'hunter2')
  await user.click(saveButton())

  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(saved()).toEqual({
    id: expect.any(String),
    name: 'Office Mac',
    host: 'mac.example.com',
    port: 5901,
    username: 'alice',
    secretId: 'vault-new',
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
  })
  expect(JSON.stringify(saved())).not.toContain('hunter2')
  expect(calls).toEqual(['setSecret', 'save vault-new'])
})

test.each(['', '   '])(
  'creating with a username of "%s" saves the secret and no username',
  async (typed) => {
    const onOpenChange = vi.fn()
    render(<VncProfileDialog open onOpenChange={onOpenChange} editing={null} />)
    fireEvent.change(field('Host'), { target: { value: 'mac' } })
    fireEvent.change(field('Username (optional)'), { target: { value: typed } })
    fireEvent.change(field('Password'), { target: { value: 'hunter2' } })
    fireEvent.click(saveButton())
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
    expect(setSecret).toHaveBeenCalledWith('hunter2')
    expect(saved()).toMatchObject({ username: null, secretId: 'vault-new' })
  },
)

test.each(['', '  '])('a name of "%s" is saved as host:port', async (typed) => {
  const onOpenChange = vi.fn()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Name'), { target: { value: typed } })
  fireEvent.change(field('Host'), { target: { value: ' mac ' } })
  fireEvent.change(field('Port'), { target: { value: '5902' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ name: 'mac:5902', host: 'mac', port: 5902 })
})

test('creating without a password stores no secret', async () => {
  const onOpenChange = vi.fn()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ secretId: null })
  expect(calls).toEqual(['save null'])
})

test('Save needs a host and a port from 1 to 65535', () => {
  render(<VncProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Host'), { target: { value: '   ' } })
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  expect(saveButton()).toBeEnabled()
  for (const port of ['', '0', '65536', '-1', '5900.5']) {
    fireEvent.change(field('Port'), { target: { value: port } })
    expect(saveButton(), port).toBeDisabled()
  }
  for (const port of ['1', '65535']) {
    fireEvent.change(field('Port'), { target: { value: port } })
    expect(saveButton(), port).toBeEnabled()
  }
})

test('editing shows the profile with an empty password field', () => {
  render(<VncProfileDialog open onOpenChange={vi.fn()} editing={desk} />)
  expect(screen.getByText('Edit VNC profile')).toBeInTheDocument()
  expect(field('Name')).toHaveValue('desk')
  expect(field('Host')).toHaveValue('10.0.0.5')
  expect(field('Port')).toHaveValue(5901)
  expect(field('Username (optional)')).toHaveValue('faye')
  expect(field('Password')).toHaveValue('')
  expect(field('Password')).toHaveAttribute('placeholder', 'unchanged')
  expect(field('Remove saved password')).not.toBeChecked()
})

test('editing with an empty password keeps the saved one', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.clear(field('Host'))
  await user.type(field('Host'), '10.0.0.6')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...desk, host: '10.0.0.6' })
  expect(calls).toEqual(['save vault-old'])
})

test('a new password replaces the saved one and leaves no vault entry behind', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'hunter3')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(setSecret).toHaveBeenCalledWith('hunter3')
  expect(saved()).toEqual({ ...desk, secretId: 'vault-new' })
  expect(calls).toEqual(['setSecret', 'save vault-new', 'deleteSecret vault-old'])
})

test('an old password that is already gone does not keep the dialog open', async () => {
  vi.mocked(deleteSecret).mockRejectedValue(new Error('notFound: secret vault-old'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'hunter3')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().secretId).toBe('vault-new')
  expect(deleteSecret).toHaveBeenCalledWith('vault-old')
})

test('Remove saved password empties and disables the field, and saving deletes the secret', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'typed')
  await user.click(field('Remove saved password'))
  expect(field('Password')).toBeDisabled()
  expect(field('Password')).toHaveValue('')

  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...desk, secretId: null })
  expect(calls).toEqual(['save null', 'deleteSecret vault-old'])
})

test('unticking Remove saved password gives the field back', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.click(field('Remove saved password'))
  await user.click(field('Remove saved password'))
  expect(field('Password')).toBeEnabled()
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['save vault-old'])
})

test('Remove saved password is only offered for a profile that has one', () => {
  const { unmount } = render(<VncProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  expect(screen.queryByLabelText('Remove saved password')).not.toBeInTheDocument()
  unmount()
  const bare = { ...desk, secretId: null }
  render(<VncProfileDialog open onOpenChange={vi.fn()} editing={bare} />)
  expect(screen.queryByLabelText('Remove saved password')).not.toBeInTheDocument()
  expect(field('Password')).toHaveAttribute('placeholder', '')
})

test('a failed save keeps the dialog open, shows the error and keeps the old password', async () => {
  save.mockRejectedValueOnce(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'hunter3')
  await user.click(saveButton())

  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(calls).toEqual(['setSecret', 'deleteSecret vault-new'])
  expect(field('Password')).toHaveValue('hunter3')

  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(screen.queryByText('io: disk full')).not.toBeInTheDocument()
  expect(calls).toEqual([
    'setSecret',
    'deleteSecret vault-new',
    'setSecret',
    'save vault-new',
    'deleteSecret vault-old',
  ])
})

test('a failed save while removing the password deletes nothing', async () => {
  save.mockRejectedValue(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.click(field('Remove saved password'))
  await user.click(saveButton())
  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(deleteSecret).not.toHaveBeenCalled()
})

test('a password the vault refuses is reported and nothing is saved', async () => {
  vi.mocked(setSecret).mockRejectedValue(new Error('vault: locked'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'hunter3')
  await user.click(saveButton())
  expect(await screen.findByText('vault: locked')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(save).not.toHaveBeenCalled()
  expect(deleteSecret).not.toHaveBeenCalled()
})

test('Save does nothing more while a save is still running', async () => {
  let finish = () => {}
  save.mockImplementation(
    (p: VncProfile) =>
      new Promise<void>((resolve) => {
        calls.push(`save ${p.secretId}`)
        finish = resolve
      }),
  )
  const onOpenChange = vi.fn()
  render(<VncProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.change(field('Password'), { target: { value: 'hunter2' } })
  fireEvent.click(saveButton())
  fireEvent.click(saveButton())
  await waitFor(() => expect(save).toHaveBeenCalled())
  expect(saveButton()).toBeDisabled()
  fireEvent.click(saveButton())

  finish()
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['setSecret', 'save vault-new'])
  expect(saveButton()).toBeEnabled()
})

test.each<[string, (user: UserEvent) => Promise<void>]>([
  ['Save', (user) => user.click(saveButton())],
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
  const user = userEvent.setup()
  render(<Reopenable editing={null} />)
  await user.type(field('Name'), 'Office Mac')
  await user.type(field('Host'), 'mac')
  fireEvent.change(field('Port'), { target: { value: '5901' } })
  await user.type(field('Username (optional)'), 'alice')
  await user.type(field('Password'), 'hunter2')

  await close(user)
  await waitFor(() => expect(screen.queryByLabelText('Password')).not.toBeInTheDocument())

  await user.click(screen.getByRole('button', { name: 'New profile' }))
  expect(field('Name')).toHaveValue('')
  expect(field('Host')).toHaveValue('')
  expect(field('Port')).toHaveValue(5900)
  expect(field('Username (optional)')).toHaveValue('')
  expect(field('Password')).toHaveValue('')
})

test('what was typed while editing is gone when the dialog opens for another profile', async () => {
  const user = userEvent.setup()
  render(<Reopenable editing={desk} />)
  await user.type(field('Password'), 'hunter2')
  await user.click(field('Remove saved password'))
  await user.click(screen.getByRole('button', { name: 'Cancel' }))

  await user.click(screen.getByRole('button', { name: 'Edit desk' }))
  expect(field('Host')).toHaveValue('10.0.0.5')
  expect(field('Password')).toHaveValue('')
  expect(field('Password')).toBeEnabled()
  expect(field('Remove saved password')).not.toBeChecked()
  await user.type(field('Password'), 'hunter3')
  await user.click(screen.getByRole('button', { name: 'Cancel' }))

  await user.click(screen.getByRole('button', { name: 'New profile' }))
  expect(field('Host')).toHaveValue('')
  expect(field('Password')).toHaveValue('')
})

test('an error is gone when the dialog is opened again', async () => {
  save.mockRejectedValue(new Error('io: disk full'))
  const user = userEvent.setup()
  render(<Reopenable editing={desk} />)
  await user.click(saveButton())
  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  await user.click(screen.getByRole('button', { name: 'Edit desk' }))
  expect(screen.queryByText('io: disk full')).not.toBeInTheDocument()
})

test('the password is gone while the closed dialog is still fading out', async () => {
  const computed = window.getComputedStyle.bind(window)
  // An exit animation keeps the content mounted after the dialog has closed.
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
  const user = userEvent.setup()
  render(<Reopenable editing={desk} />)
  await user.type(field('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(field('Password')).toHaveValue('')
  expect(field('Host')).toHaveValue('')
})
