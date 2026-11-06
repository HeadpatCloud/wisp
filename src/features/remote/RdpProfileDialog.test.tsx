import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { RdpProfile } from '@/bindings'

vi.mock('@/lib/vault', () => ({
  setSecret: vi.fn(),
  deleteSecret: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(),
}))

import { open as pickFolder } from '@tauri-apps/plugin-dialog'
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
  audio: true,
  sharedFolder: null,
  gateway: null,
  icon: { kind: 'builtin', name: 'cloud' },
  order: 4,
}

// A profile that reaches its server through a gateway with a login of its own.
const gated: RdpProfile = {
  ...desk,
  gateway: {
    host: 'gw.corp',
    port: 8443,
    sameLogin: false,
    username: 'gw-faye',
    secretId: 'gw-old',
  },
}

const GATEWAY_PASSWORD = 'opensesame'

const save = vi.fn()
// Every vault and store call in the order it was made. The new id never embeds the password.
let calls: string[] = []

beforeEach(() => {
  vi.resetAllMocks()
  calls = []
  vi.mocked(setSecret).mockImplementation(async (secret) => {
    calls.push('setSecret')
    return secret === GATEWAY_PASSWORD ? 'gw-new' : 'vault-new'
  })
  vi.mocked(deleteSecret).mockImplementation(async (id) => {
    calls.push(`deleteSecret ${id}`)
  })
  save.mockImplementation(async (p: RdpProfile) => {
    calls.push(
      p.gateway ? `save ${p.secretId} gateway ${p.gateway.secretId}` : `save ${p.secretId}`,
    )
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
      <button type="button" onClick={() => reopen(gated)}>
        Edit gated
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
    audio: true,
    sharedFolder: null,
    gateway: null,
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

const sound = () => field('Play sound from the remote computer')
const shareBox = () => field('Share a folder')
const gatewayBox = () => field('Connect through an RD Gateway')
const sameLogin = () => field('Use the same login for the gateway')
const browse = () => screen.getByRole('button', { name: 'Browse…' })
const SHARE_RULE = 'A share name has 1 to 7 letters, digits, - or _.'

async function pick(user: UserEvent, folder: string | null) {
  vi.mocked(pickFolder).mockResolvedValue(folder)
  await user.click(browse())
  await waitFor(() => expect(pickFolder).toHaveBeenCalledWith({ directory: true }))
}

test('a new profile plays sound, shares no folder and uses no gateway', async () => {
  const onOpenChange = vi.fn()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  expect(sound()).toBeChecked()
  expect(shareBox()).not.toBeChecked()
  expect(gatewayBox()).not.toBeChecked()
  expect(screen.queryByLabelText('Folder')).not.toBeInTheDocument()
  expect(screen.queryByLabelText('Share name')).not.toBeInTheDocument()
  expect(screen.queryByLabelText('Gateway host')).not.toBeInTheDocument()
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  fireEvent.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toMatchObject({ audio: true, sharedFolder: null, gateway: null })
})

test('a profile without sound is saved without it and shown so', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  const { unmount } = render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={desk} />)
  await user.click(sound())
  expect(sound()).not.toBeChecked()
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...desk, audio: false })
  unmount()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={{ ...desk, audio: false }} />)
  expect(sound()).not.toBeChecked()
})

test('sound, folder and gateway come below the keyboard layout, in this order', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  await user.click(gatewayBox())
  await user.click(sameLogin())
  const shown = [
    layoutTrigger(),
    sound(),
    shareBox(),
    field('Folder'),
    browse(),
    field('Share name'),
    screen.getByText(/^Shows up as/),
    gatewayBox(),
    field('Gateway host'),
    field('Gateway port'),
    sameLogin(),
    field('Gateway username'),
    field('Gateway password'),
  ]
  for (let i = 1; i < shown.length; i++) {
    const position = shown[i - 1].compareDocumentPosition(shown[i])
    expect(position & Node.DOCUMENT_POSITION_FOLLOWING, `${i}`).toBeTruthy()
  }
})

test('sharing a folder shows a read-only folder field, a share name and where it shows up', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  expect(field('Folder')).toHaveValue('')
  expect(field('Folder')).toHaveAttribute('readonly')
  expect(field('Share name')).toHaveValue('')
  await user.type(field('Share name'), 'docs')
  expect(screen.getByText('Shows up as \\\\tsclient\\docs')).toBeInTheDocument()
  expect(field('Share name')).toHaveAccessibleDescription('Shows up as \\\\tsclient\\docs')
})

test.each([
  ['C:\\Users\\me\\My Documents', 'MyDocum'],
  ['/home/me/ü', 'share'],
  ['/srv/my-share_2024', 'my-shar'],
  ['D:\\', 'D'],
])('picking %s fills an empty share name with %s', async (folder, name) => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  await pick(user, folder)
  await waitFor(() => expect(field('Folder')).toHaveValue(folder))
  expect(field('Share name')).toHaveValue(name)
})

test('picking a folder keeps a share name that is already there', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  await user.type(field('Share name'), 'mine')
  await pick(user, 'C:\\Users\\me\\My Documents')
  await waitFor(() => expect(field('Folder')).toHaveValue('C:\\Users\\me\\My Documents'))
  expect(field('Share name')).toHaveValue('mine')
})

test('a folder picker that is cancelled changes nothing', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  await pick(user, '/home/me/docs')
  await waitFor(() => expect(field('Folder')).toHaveValue('/home/me/docs'))
  fireEvent.change(field('Share name'), { target: { value: '' } })
  vi.mocked(pickFolder).mockClear()
  await pick(user, null)
  expect(field('Folder')).toHaveValue('/home/me/docs')
  expect(field('Share name')).toHaveValue('')
})

test.each(['toolong8', 'wi sp', 'dö', 'a.b', 'a\\b'])(
  'the share name "%s" is refused',
  async (typed) => {
    const user = userEvent.setup()
    render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
    await user.click(shareBox())
    fireEvent.change(field('Share name'), { target: { value: typed } })
    expect(screen.getByText(SHARE_RULE)).toBeInTheDocument()
    expect(field('Share name')).toHaveAttribute('aria-invalid', 'true')
    expect(field('Share name')).toHaveAccessibleDescription(
      `Shows up as \\\\tsclient\\${typed} ${SHARE_RULE}`,
    )
  },
)

test.each(['', '  '])(
  'an empty share name shows no rule but keeps Save disabled',
  async (typed) => {
    const user = userEvent.setup()
    render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
    fireEvent.change(field('Host'), { target: { value: 'mac' } })
    await user.click(shareBox())
    await pick(user, '/home/me/docs')
    await waitFor(() => expect(field('Share name')).toHaveValue('docs'))
    fireEvent.change(field('Share name'), { target: { value: typed } })
    expect(screen.queryByText(SHARE_RULE)).not.toBeInTheDocument()
    expect(field('Share name')).not.toHaveAttribute('aria-invalid', 'true')
    expect(field('Share name')).toHaveAccessibleDescription('Shows up as \\\\tsclient\\')
    expect(saveButton()).toBeDisabled()
  },
)

test.each(['a', 'Docs-_1', '1234567', ' docs '])('the share name "%s" is taken', async (typed) => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(shareBox())
  fireEvent.change(field('Share name'), { target: { value: typed } })
  expect(screen.queryByText(SHARE_RULE)).not.toBeInTheDocument()
  expect(field('Share name')).not.toHaveAttribute('aria-invalid', 'true')
})

test('Save needs a folder and a valid share name while a folder is shared', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  expect(saveButton()).toBeEnabled()
  await user.click(shareBox())
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Share name'), { target: { value: 'docs' } })
  expect(saveButton()).toBeDisabled()
  await pick(user, '/home/me/docs')
  await waitFor(() => expect(saveButton()).toBeEnabled())
  for (const name of ['', 'toolong8', 'my docs']) {
    fireEvent.change(field('Share name'), { target: { value: name } })
    expect(saveButton(), name).toBeDisabled()
  }
  await user.click(shareBox())
  expect(saveButton()).toBeEnabled()
})

test('a shared folder is saved with its path and its name', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(shareBox())
  await pick(user, 'C:\\Users\\me\\My Documents')
  await waitFor(() => expect(saveButton()).toBeEnabled())
  fireEvent.change(field('Share name'), { target: { value: ' docs ' } })
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().sharedFolder).toEqual({ path: 'C:\\Users\\me\\My Documents', name: 'docs' })
})

test('a shared folder is shown when editing, and unticking it saves no folder', async () => {
  const shared = { ...desk, sharedFolder: { path: '/home/faye/docs', name: 'docs' } }
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={shared} />)
  expect(shareBox()).toBeChecked()
  expect(field('Folder')).toHaveValue('/home/faye/docs')
  expect(field('Share name')).toHaveValue('docs')
  await user.click(shareBox())
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...shared, sharedFolder: null })
})

test('a gateway starts on port 443 with the same login', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(gatewayBox())
  expect(field('Gateway host')).toHaveValue('')
  expect(field('Gateway port')).toHaveValue(443)
  expect(sameLogin()).toBeChecked()
  expect(screen.queryByLabelText('Gateway username')).not.toBeInTheDocument()
  expect(screen.queryByLabelText('Gateway password')).not.toBeInTheDocument()
  fireEvent.change(field('Gateway host'), { target: { value: ' gw.corp ' } })
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().gateway).toEqual({
    host: 'gw.corp',
    port: 443,
    sameLogin: true,
    username: '',
    secretId: null,
  })
  expect(calls).toEqual(['save null gateway null'])
})

test('Save needs a gateway host and a gateway port from 1 to 65535', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(gatewayBox())
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Gateway host'), { target: { value: '   ' } })
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Gateway host'), { target: { value: 'gw' } })
  expect(saveButton()).toBeEnabled()
  for (const port of ['', '0', '65536', '-1', '443.5']) {
    fireEvent.change(field('Gateway port'), { target: { value: port } })
    expect(saveButton(), port).toBeDisabled()
  }
  for (const port of ['1', '65535']) {
    fireEvent.change(field('Gateway port'), { target: { value: port } })
    expect(saveButton(), port).toBeEnabled()
  }
  fireEvent.change(field('Gateway host'), { target: { value: '' } })
  await user.click(gatewayBox())
  expect(saveButton()).toBeEnabled()
})

test('Save needs a gateway user name for a login of its own', async () => {
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(gatewayBox())
  fireEvent.change(field('Gateway host'), { target: { value: 'gw' } })
  await user.click(sameLogin())
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Gateway username'), { target: { value: '  ' } })
  expect(saveButton()).toBeDisabled()
  fireEvent.change(field('Gateway username'), { target: { value: 'gw-faye' } })
  expect(saveButton()).toBeEnabled()
  fireEvent.change(field('Gateway username'), { target: { value: '' } })
  await user.click(sameLogin())
  expect(saveButton()).toBeEnabled()
})

test('a gateway login of its own stores its password in the vault', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(gatewayBox())
  fireEvent.change(field('Gateway host'), { target: { value: 'gw.corp' } })
  fireEvent.change(field('Gateway port'), { target: { value: '8443' } })
  await user.click(sameLogin())
  fireEvent.change(field('Gateway username'), { target: { value: ' gw-faye ' } })
  expect(field('Gateway password')).toHaveAttribute('placeholder', '')
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(setSecret).toHaveBeenCalledWith(GATEWAY_PASSWORD)
  expect(saved().gateway).toEqual({
    host: 'gw.corp',
    port: 8443,
    sameLogin: false,
    username: 'gw-faye',
    secretId: 'gw-new',
  })
  expect(JSON.stringify(saved())).not.toContain(GATEWAY_PASSWORD)
  expect(calls).toEqual(['setSecret', 'save null gateway gw-new'])
})

test('both passwords are stored before the save', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.type(field('Password'), 'hunter2')
  await user.click(gatewayBox())
  fireEvent.change(field('Gateway host'), { target: { value: 'gw.corp' } })
  await user.click(sameLogin())
  fireEvent.change(field('Gateway username'), { target: { value: 'gw-faye' } })
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['setSecret', 'setSecret', 'save vault-new gateway gw-new'])
})

test('a gateway password typed before the same login was chosen is not stored', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={null} />)
  fireEvent.change(field('Host'), { target: { value: 'mac' } })
  await user.click(gatewayBox())
  fireEvent.change(field('Gateway host'), { target: { value: 'gw.corp' } })
  await user.click(sameLogin())
  fireEvent.change(field('Gateway username'), { target: { value: 'gw-faye' } })
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(sameLogin())
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(setSecret).not.toHaveBeenCalled()
  expect(saved().gateway).toMatchObject({ sameLogin: true, username: '', secretId: null })
})

test('editing shows the gateway with an empty gateway password field', () => {
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={gated} />)
  expect(gatewayBox()).toBeChecked()
  expect(field('Gateway host')).toHaveValue('gw.corp')
  expect(field('Gateway port')).toHaveValue(8443)
  expect(sameLogin()).not.toBeChecked()
  expect(field('Gateway username')).toHaveValue('gw-faye')
  expect(field('Gateway password')).toHaveValue('')
  expect(field('Gateway password')).toHaveAttribute('placeholder', 'unchanged')
  expect(field('Remove saved gateway password')).not.toBeChecked()
})

test('editing with an empty gateway password keeps the saved one', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual(gated)
  expect(calls).toEqual(['save vault-old gateway gw-old'])
})

test('a new gateway password replaces the saved one and leaves no vault entry behind', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['setSecret', 'save vault-old gateway gw-new', 'deleteSecret gw-old'])
})

test('two new passwords replace both saved ones after the save', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Password'), 'hunter3')
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual([
    'setSecret',
    'setSecret',
    'save vault-new gateway gw-new',
    'deleteSecret vault-old',
    'deleteSecret gw-old',
  ])
})

test('an old gateway password that is already gone does not keep the dialog open', async () => {
  vi.mocked(deleteSecret).mockRejectedValue(new Error('notFound: secret gw-old'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(deleteSecret).toHaveBeenCalledWith('gw-old')
})

test('Remove saved gateway password empties and disables the field, and saving deletes the secret', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Gateway password'), 'typed')
  await user.click(field('Remove saved gateway password'))
  expect(field('Gateway password')).toBeDisabled()
  expect(field('Gateway password')).toHaveValue('')
  expect(field('Gateway password')).toHaveAttribute('placeholder', '')
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().gateway?.secretId).toBeNull()
  expect(calls).toEqual(['save vault-old gateway null', 'deleteSecret gw-old'])
})

test('unticking Remove saved gateway password gives the field back', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.click(field('Remove saved gateway password'))
  await user.click(field('Remove saved gateway password'))
  expect(field('Gateway password')).toBeEnabled()
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual(['save vault-old gateway gw-old'])
})

test('Remove saved gateway password is only offered for a gateway that has one', async () => {
  const user = userEvent.setup()
  const { unmount } = render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={null} />)
  await user.click(gatewayBox())
  await user.click(sameLogin())
  expect(screen.queryByLabelText('Remove saved gateway password')).not.toBeInTheDocument()
  unmount()
  const bare: RdpProfile = {
    ...desk,
    gateway: { host: 'gw.corp', port: 443, sameLogin: false, username: 'gw', secretId: null },
  }
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={bare} />)
  expect(screen.queryByLabelText('Remove saved gateway password')).not.toBeInTheDocument()
  expect(field('Gateway password')).toHaveAttribute('placeholder', '')
})

test('a failed save removes the new gateway password and keeps the old one', async () => {
  save.mockRejectedValueOnce(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())

  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(calls).toEqual(['setSecret', 'deleteSecret gw-new'])
  expect(field('Gateway password')).toHaveValue(GATEWAY_PASSWORD)

  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(calls).toEqual([
    'setSecret',
    'deleteSecret gw-new',
    'setSecret',
    'save vault-old gateway gw-new',
    'deleteSecret gw-old',
  ])
})

test('a failed save removes both new passwords', async () => {
  save.mockRejectedValue(new Error('io: disk full'))
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={vi.fn()} editing={gated} />)
  await user.type(field('Password'), 'hunter3')
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  expect(calls).toEqual(['setSecret', 'setSecret', 'deleteSecret vault-new', 'deleteSecret gw-new'])
})

test('a gateway password the vault refuses removes the new RDP password and saves nothing', async () => {
  vi.mocked(setSecret).mockImplementation(async (secret) => {
    calls.push('setSecret')
    if (secret === GATEWAY_PASSWORD) throw new Error('vault: locked')
    return 'vault-new'
  })
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.type(field('Password'), 'hunter3')
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(saveButton())
  expect(await screen.findByText('vault: locked')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(save).not.toHaveBeenCalled()
  expect(calls).toEqual(['setSecret', 'setSecret', 'deleteSecret vault-new'])
})

test.each<[string, (user: UserEvent) => Promise<void>]>([
  [
    'removing the saved gateway password',
    (user) => user.click(field('Remove saved gateway password')),
  ],
  ['turning the gateway off', (user) => user.click(gatewayBox())],
  ['choosing the same login', (user) => user.click(sameLogin())],
])('a failed save while %s deletes nothing', async (_how, change) => {
  save.mockRejectedValue(new Error('io: disk full'))
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await change(user)
  await user.click(saveButton())
  expect(await screen.findByText('io: disk full')).toBeInTheDocument()
  expect(onOpenChange).not.toHaveBeenCalled()
  expect(deleteSecret).not.toHaveBeenCalled()
})

test('turning the gateway off deletes its password after the save', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.click(gatewayBox())
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...gated, gateway: null })
  expect(calls).toEqual(['save vault-old', 'deleteSecret gw-old'])
})

test('choosing the same login deletes the gateway password after the save', async () => {
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={gated} />)
  await user.click(sameLogin())
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved().gateway).toEqual({
    host: 'gw.corp',
    port: 8443,
    sameLogin: true,
    username: '',
    secretId: null,
  })
  expect(calls).toEqual(['save vault-old gateway null', 'deleteSecret gw-old'])
})

test('a profile saved before sound, folders and gateways opens with sound on and is saved with all three', async () => {
  const older: RdpProfile = {
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
  const onOpenChange = vi.fn()
  const user = userEvent.setup()
  render(<RdpProfileDialog open onOpenChange={onOpenChange} editing={older} />)
  expect(sound()).toBeChecked()
  expect(shareBox()).not.toBeChecked()
  expect(gatewayBox()).not.toBeChecked()
  await user.click(saveButton())
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  expect(saved()).toEqual({ ...older, audio: true, sharedFolder: null, gateway: null })
  expect(calls).toEqual(['save vault-old'])
})

test('closing forgets the sound, folder and gateway settings that were changed', async () => {
  const user = userEvent.setup()
  render(<Reopenable editing={null} />)
  await user.click(sound())
  await user.click(shareBox())
  await pick(user, '/home/me/docs')
  await waitFor(() => expect(field('Share name')).toHaveValue('docs'))
  await user.click(gatewayBox())
  fireEvent.change(field('Gateway host'), { target: { value: 'gw' } })
  fireEvent.change(field('Gateway port'), { target: { value: '8443' } })
  await user.click(sameLogin())
  fireEvent.change(field('Gateway username'), { target: { value: 'gw-faye' } })
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  await waitFor(() => expect(screen.queryByLabelText('Host')).not.toBeInTheDocument())

  await user.click(screen.getByRole('button', { name: 'New profile' }))
  expect(sound()).toBeChecked()
  expect(shareBox()).not.toBeChecked()
  expect(gatewayBox()).not.toBeChecked()
  await user.click(shareBox())
  await user.click(gatewayBox())
  expect(field('Folder')).toHaveValue('')
  expect(field('Share name')).toHaveValue('')
  expect(field('Gateway host')).toHaveValue('')
  expect(field('Gateway port')).toHaveValue(443)
  expect(sameLogin()).toBeChecked()
})

test('what was typed for the gateway password is gone when the dialog opens again', async () => {
  const user = userEvent.setup()
  render(<Reopenable editing={gated} />)
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(field('Remove saved gateway password'))
  await user.click(screen.getByRole('button', { name: 'Cancel' }))

  await user.click(screen.getByRole('button', { name: 'Edit gated' }))
  expect(field('Gateway password')).toHaveValue('')
  expect(field('Gateway password')).toBeEnabled()
  expect(field('Remove saved gateway password')).not.toBeChecked()
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(screen.getByRole('button', { name: 'Cancel' }))

  await user.click(screen.getByRole('button', { name: 'New profile' }))
  await user.click(gatewayBox())
  await user.click(sameLogin())
  expect(field('Gateway password')).toHaveValue('')
})

test('the gateway password is gone while the closed dialog is still fading out', async () => {
  keepClosingDialog()
  const user = userEvent.setup()
  render(<Reopenable editing={gated} />)
  await user.type(field('Gateway password'), GATEWAY_PASSWORD)
  await user.click(field('Remove saved gateway password'))
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )
  expect(field('Gateway password')).toHaveValue('')
  expect(field('Remove saved gateway password')).not.toBeChecked()
  expect(field('Gateway host')).toHaveValue('gw.corp')
})
