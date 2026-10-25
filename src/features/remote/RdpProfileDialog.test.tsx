import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { RdpProfile } from '@/bindings'

vi.mock('@/lib/vault', () => ({
  setSecret: vi.fn(),
  deleteSecret: vi.fn(),
}))

import { deleteSecret, setSecret } from '@/lib/vault'
import { useRdpProfileStore } from '@/stores/rdpProfileStore'
import { RdpProfileDialog } from './RdpProfileDialog'

const desk: RdpProfile = {
  id: 'rdp-1',
  name: 'desk',
  host: '10.0.0.5',
  port: 3390,
  username: 'faye',
  domain: 'CORP',
  secretId: 'vault-old',
  keyboardLayout: 0x407,
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
  save.mockImplementation(async (p: RdpProfile) => {
    calls.push(`save ${p.secretId}`)
  })
  useRdpProfileStore.setState({ profiles: [], loaded: true, save } as never)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

const field = (label: string) => screen.getByLabelText(label)
const saveButton = () => screen.getByRole('button', { name: 'Save' })
const saved = () => save.mock.calls.at(-1)?.[0] as RdpProfile

function Reopenable({ editing }: { editing: RdpProfile | null }) {
  const [open, setOpen] = useState(true)
  const [target, setTarget] = useState(editing)
  const reopen = (next: RdpProfile | null) => {
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
      <RdpProfileDialog open={open} onOpenChange={setOpen} editing={target} />
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

test('creating stores the password in the vault and saves the profile', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  expect(screen.getByText('New RDP profile')).toBeInTheDocument()
  expect(field('Port')).toHaveValue(3389)
  fireEvent.change(field('Name'), { target: { value: 'Office Mac' } })
  fireEvent.change(field('Host'), { target: { value: '  mac.example.com ' } })
  fireEvent.change(field('Port'), { target: { value: '3390' } })
  fireEvent.change(field('Username'), { target: { value: ' alice ' } })
  fireEvent.change(field('Domain (optional)'), { target: { value: ' CORP ' } })
  await user.type(field('Password'), 'hunter2')
  await user.click(saveButton())

  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(setSecret).toHaveBeenCalledWith('hunter2')
  expect(saved()).toEqual({
    id: expect.any(String),
    name: 'Office Mac',
    host: 'mac.example.com',
    port: 3390,
    username: 'alice',
    domain: 'CORP',
    secretId: 'vault-new',
    keyboardLayout: null,
    icon: { kind: 'builtin', name: 'server' },
    order: 0,
  })
  expect(JSON.stringify(saved())).not.toContain('hunter2')
  expect(calls).toEqual(['setSecret', 'save vault-new'])
})

test.each(['', '   '])('a domain of "%s" is saved as no domain', async (typed) => {
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.change(field('Domain (optional)'), { target: { value: typed } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ domain: null, username: '' })
})

test.each(['CORP\\alice', 'alice@corp.example'])(
  'a domain next to the user name %s is refused',
  (typed) => {
    render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
    fireEvent.change(field('Host'), { target: { value: 'mac' } })
    fireEvent.change(field('Username'), { target: { value: typed } })
    expect(saveButton()).toBeEnabled()
    fireEvent.change(field('Domain (optional)'), { target: { value: 'CORP' } })
    expect(
      screen.getByText('Put the domain either in the user name or in the Domain field.'),
    ).toBeInTheDocument()
    expect(saveButton()).toBeDisabled()
    fireEvent.change(field('Domain (optional)'), { target: { value: '  ' } })
    expect(saveButton()).toBeEnabled()
  },
)

test('the domain rule describes the Domain field and marks it invalid', () => {
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  fireEvent.change(field('Username'), { target: { value: 'alice@corp.example' } })
  fireEvent.change(field('Domain (optional)'), { target: { value: 'CORP' } })
  expect(field('Domain (optional)')).toHaveAccessibleDescription(
    'Put the domain either in the user name or in the Domain field.',
  )
  expect(field('Domain (optional)')).toHaveAttribute('aria-invalid', 'true')

  fireEvent.change(field('Domain (optional)'), { target: { value: '' } })
  expect(field('Domain (optional)')).not.toHaveAccessibleDescription(
    'Put the domain either in the user name or in the Domain field.',
  )
  expect(field('Domain (optional)')).not.toHaveAttribute('aria-invalid', 'true')
})

test('a user name with a domain in it is saved as typed', async () => {
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.change(field('Username'), { target: { value: ' CORP\\alice ' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ username: 'CORP\\alice', domain: null })
})

const layoutTrigger = () => screen.getByRole('combobox', { name: 'Keyboard layout' })

test('the keyboard layout is Automatic until one is picked, and is saved as its number', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  expect(layoutTrigger()).toHaveTextContent('Automatic')
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(layoutTrigger())
  await user.click(await screen.findByRole('option', { name: 'German (Switzerland)' }))
  expect(layoutTrigger()).toHaveTextContent('German (Switzerland)')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().keyboardLayout).toBe(0x807)
})

test('the layouts on offer are the listed ones', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(layoutTrigger())
  const names = (await screen.findAllByRole('option')).map((o) => o.textContent)
  expect(names).toEqual([
    'Automatic',
    'English (US)',
    'English (US International)',
    'English (UK)',
    'German',
    'German (Switzerland)',
    'French',
    'French (Belgium)',
    'French (Switzerland)',
    'Dutch',
    'Belgian (Period)',
    'Spanish',
    'Italian',
    'Portuguese',
    'Portuguese (Brazil)',
    'Danish',
    'Norwegian',
    'Swedish',
    'Finnish',
    'Polish (Programmers)',
    'Czech',
    'Hungarian',
    'Turkish Q',
    'Russian',
    'Japanese',
  ])
})

test.each([
  ['English (US)', 0x409],
  ['English (US International)', 0x20409],
  ['English (UK)', 0x809],
  ['German', 0x407],
  ['German (Switzerland)', 0x807],
  ['French', 0x40c],
  ['French (Belgium)', 0x80c],
  ['French (Switzerland)', 0x100c],
  ['Dutch', 0x413],
  ['Belgian (Period)', 0x813],
  ['Spanish', 0x40a],
  ['Italian', 0x410],
  ['Portuguese', 0x816],
  ['Portuguese (Brazil)', 0x416],
  ['Danish', 0x406],
  ['Norwegian', 0x414],
  ['Swedish', 0x41d],
  ['Finnish', 0x40b],
  ['Polish (Programmers)', 0x415],
  ['Czech', 0x405],
  ['Hungarian', 0x40e],
  ['Turkish Q', 0x41f],
  ['Russian', 0x419],
  ['Japanese', 0x411],
])('%s is saved as %i', async (name, number) => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(layoutTrigger())
  await user.click(await screen.findByRole('option', { name }))
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().keyboardLayout).toBe(number)
})

test('Automatic after a layout was set is saved as no layout', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  expect(layoutTrigger()).toHaveTextContent('German')
  await user.click(layoutTrigger())
  await user.click(await screen.findByRole('option', { name: 'Automatic' }))
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...desk, keyboardLayout: null })
})

test('a stored layout that is not listed is shown by its number and kept on save', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(
    <RdpProfileDialog
      open
      onOpenChange={onOpenChange}
      editing={{ ...desk, keyboardLayout: 0x40d }}
    />,
  )
  expect(layoutTrigger()).toHaveTextContent('0x40d')
  await user.click(layoutTrigger())
  const names = (await screen.findAllByRole('option')).map((o) => o.textContent)
  expect(names).toHaveLength(26)
  expect(names.at(-1)).toBe('0x40d')
  await user.keyboard('{Escape}')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...desk, keyboardLayout: 0x40d })
})

test('a stored layout that is listed adds no extra choice', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={desk} />)
  await user.click(layoutTrigger())
  expect(await screen.findAllByRole('option')).toHaveLength(25)
})

test.each(['', '  '])('a name of "%s" is saved as host:port', async (typed) => {
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Name'), { target: { value: typed } })
  fireEvent.change(field('Host'), { target: { value: ' mac ' } })
  fireEvent.change(field('Port'), { target: { value: '3391' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ name: 'mac:3391', host: 'mac', port: 3391 })
})

test('creating without a password stores no secret', async () => {
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ secretId: null })
  expect(calls).toEqual(['save null'])
})

test('Save needs a host and a port from 1 to 65535', () => {
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Host'), { target: { value: '   ' } })
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  expect(saveButton()).toBeEnabled()
  for (const port of ['', '0', '65536', '-1', '3389.5']) {
    fireEvent.change(field('Port'), { target: { value: port } })
    expect(saveButton(), port).toBeDisabled()
  }
  for (const port of ['1', '65535']) {
    fireEvent.change(field('Port'), { target: { value: port } })
    expect(saveButton(), port).toBeEnabled()
  }
})

test('editing shows the profile with an empty password field', () => {
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={desk} />)
  expect(screen.getByText('Edit RDP profile')).toBeInTheDocument()
  expect(field('Name')).toHaveValue('desk')
  expect(field('Host')).toHaveValue('10.0.0.5')
  expect(field('Port')).toHaveValue(3390)
  expect(field('Username')).toHaveValue('faye')
  expect(field('Domain (optional)')).toHaveValue('CORP')
  expect(layoutTrigger()).toHaveTextContent('German')
  expect(field('Password')).toHaveValue('')
  expect(field('Password')).toHaveAttribute('placeholder', 'unchanged')
  expect(field('Remove saved password')).not.toBeChecked()
})

test('editing with an empty password keeps the saved one', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.type(field('Password'), 'hunter3')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().secretId).toBe('vault-new')
  expect(deleteSecret).toHaveBeenCalledWith('vault-old')
})

test('Remove saved password empties and disables the field, and saving deletes the secret', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.click(field('Remove saved password'))
  await user.click(field('Remove saved password'))
  expect(field('Password')).toBeEnabled()
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['save vault-old'])
})

test('Remove saved password is only offered for a profile that has one', () => {
  const { unmount } = render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  expect(screen.queryByLabelText('Remove saved password')).not.toBeInTheDocument()
  unmount()
  const bare = { ...desk, secretId: null }
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={bare} />)
  expect(screen.queryByLabelText('Remove saved password')).not.toBeInTheDocument()
  expect(field('Password')).toHaveAttribute('placeholder', '')
})

test('a failed save keeps the dialog open, shows the error and keeps the old password', async () => {
  save.mockRejectedValueOnce(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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

test('a failed save is announced as an alert', async () => {
  save.mockRejectedValueOnce(new Error('io: disk full'))
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={desk} />)
  expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  await user.click(saveButton())

  expect(await screen.findByRole('alert')).toHaveTextContent('io: disk full')
})

test('a failed save while removing the password deletes nothing', async () => {
  save.mockRejectedValue(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
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
    (p: RdpProfile) =>
      new Promise<void>((resolve) => {
        calls.push(`save ${p.secretId}`)
        finish = resolve
      }),
  )
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
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
  fireEvent.change(field('Port'), { target: { value: '3390' } })
  await user.type(field('Username'), 'alice')
  await user.type(field('Domain (optional)'), 'CORP')
  await user.click(layoutTrigger())
  await user.click(await screen.findByRole('option', { name: 'Japanese' }))
  await user.type(field('Password'), 'hunter2')

  await close(user)
  await waitFor(() => expect(screen.queryByLabelText('Password')).not.toBeInTheDocument())

  await user.click(screen.getByRole('button', { name: 'New profile' }))
  expect(field('Name')).toHaveValue('')
  expect(field('Host')).toHaveValue('')
  expect(field('Port')).toHaveValue(3389)
  expect(field('Username')).toHaveValue('')
  expect(field('Domain (optional)')).toHaveValue('')
  expect(layoutTrigger()).toHaveTextContent('Automatic')
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
  keepClosingDialog()
  const user = userEvent.setup()
  render(<Reopenable editing={desk} />)
  await user.type(field('Password'), 'hunter2')
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(field('Password')).toHaveValue('')

  fireEvent.click(screen.getByRole('button', { name: 'New profile', hidden: true }))
  expect(field('Host')).toHaveValue('')
})

test('the other fields stay as typed while the closed dialog is still fading out', async () => {
  keepClosingDialog()
  save.mockRejectedValueOnce(new Error('io: disk full'))
  const user = userEvent.setup()
  render(<Reopenable editing={desk} />)
  await user.clear(field('Name'))
  await user.type(field('Name'), 'Office Mac')
  await user.clear(field('Host'))
  await user.type(field('Host'), 'mac')
  fireEvent.change(field('Port'), { target: { value: '3391' } })
  await user.clear(field('Username'))
  await user.type(field('Username'), 'alice')
  await user.click(saveButton())
  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  await user.type(field('Password'), 'hunter2')
  await user.click(field('Remove saved password'))
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(field('Name')).toHaveValue('Office Mac')
  expect(field('Host')).toHaveValue('mac')
  expect(field('Port')).toHaveValue(3391)
  expect(field('Username')).toHaveValue('alice')
  expect(field('Password')).toHaveValue('')
  expect(field('Remove saved password')).not.toBeChecked()
  expect(screen.queryByText('io: disk full')).not.toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: 'Edit desk', hidden: true }))
  expect(field('Name')).toHaveValue('desk')
  expect(field('Host')).toHaveValue('10.0.0.5')
  expect(field('Port')).toHaveValue(3390)
  expect(field('Username')).toHaveValue('faye')
})

async function saveTwiceAcrossTheClose() {
  fireEvent.click(saveButton())
  const content = document.querySelector('[data-slot="dialog-content"]')
  await waitFor(() => expect(content).toHaveAttribute('data-state', 'closed'))
  expect(saveButton()).toBeDisabled()
  fireEvent.click(saveButton())
  await new Promise((resolve) => setTimeout(resolve, 20))
}

test('a second click on Save while the dialog closes saves a new profile once', async () => {
  keepClosingDialog()
  render(<Reopenable editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.change(field('Password'), { target: { value: 'hunter2' } })
  await saveTwiceAcrossTheClose()

  expect(calls).toEqual(['setSecret', 'save vault-new'])
})

test('a second click on Save while the dialog closes leaves an edited profile its new password', async () => {
  keepClosingDialog()
  render(<Reopenable editing={desk} />)
  fireEvent.change(field('Password'), { target: { value: 'hunter3' } })
  await saveTwiceAcrossTheClose()

  expect(calls).toEqual(['setSecret', 'save vault-new', 'deleteSecret vault-old'])
  expect(saved()).toEqual({ ...desk, secretId: 'vault-new' })
})
