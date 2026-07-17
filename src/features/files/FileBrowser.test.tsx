import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import type { SftpEntry } from '@/bindings'

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => {}) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: vi.fn(),
  message: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}))

import { type FileBackend, FileBrowser } from './FileBrowser'

const entry = (name: string, isDir = false, size = 0): SftpEntry => ({
  name,
  path: `/${name}`,
  isDir,
  isSymlink: false,
  size,
  modified: null,
})

const files = [entry('a.txt', false, 10), entry('b.txt', false, 20), entry('c.txt', false, 30)]

function makeBackend(): FileBackend {
  return {
    list: vi.fn().mockResolvedValue(files),
    exists: vi.fn().mockResolvedValue(false),
    mkdir: vi.fn(),
    rename: vi.fn().mockResolvedValue(undefined),
    remove: vi.fn().mockResolvedValue(undefined),
    upload: vi.fn(),
    download: vi.fn(),
    downloadTo: vi.fn(),
  }
}

const row = (name: string) => screen.getByText(name).closest('li') as HTMLElement

beforeEach(() => {
  vi.clearAllMocks()
})

test('click selects, ctrl+click toggles, shift+click ranges', async () => {
  const user = userEvent.setup()
  render(<FileBrowser backend={makeBackend()} />)
  await user.click(await screen.findByText('a.txt'))
  expect(row('a.txt')).toHaveAttribute('data-selected')
  expect(screen.getByText('1 selected · 10 B')).toBeInTheDocument()

  await user.keyboard('{Control>}')
  await user.click(screen.getByText('c.txt'))
  await user.keyboard('{/Control}')
  expect(screen.getByText('2 selected · 40 B')).toBeInTheDocument()

  await user.click(screen.getByText('a.txt'))
  await user.keyboard('{Shift>}')
  await user.click(screen.getByText('c.txt'))
  await user.keyboard('{/Shift}')
  expect(screen.getByText('3 selected · 60 B')).toBeInTheDocument()
  expect(row('b.txt')).toHaveAttribute('data-selected')
})

test('ctrl+a selects all and escape clears', async () => {
  const user = userEvent.setup()
  render(<FileBrowser backend={makeBackend()} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  expect(screen.getByText('3 selected · 60 B')).toBeInTheDocument()
  await user.keyboard('{Escape}')
  expect(screen.queryByText('3 selected · 60 B')).not.toBeInTheDocument()
})

test('bulk delete removes every selected entry after confirming', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  await user.click(screen.getByRole('button', { name: 'Delete' }))
  expect(await screen.findByText('Delete 3 items?')).toBeInTheDocument()
  await user.click(screen.getAllByRole('button', { name: 'Delete' }).at(-1) as HTMLElement)
  await waitFor(() => expect(backend.remove).toHaveBeenCalledTimes(3))
  expect(backend.remove).toHaveBeenCalledWith('/a.txt', false)
})

test('move renames each selected entry into the destination', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  render(<FileBrowser backend={backend} />)
  await user.click(await screen.findByText('a.txt'))
  await user.keyboard('{Control>}')
  await user.click(screen.getByText('b.txt'))
  await user.keyboard('{/Control}')
  await user.click(screen.getByRole('button', { name: 'Move…' }))
  const input = await screen.findByRole('textbox')
  await user.clear(input)
  await user.type(input, '/archive')
  await user.click(screen.getByRole('button', { name: 'Move' }))
  await waitFor(() => expect(backend.rename).toHaveBeenCalledTimes(2))
  expect(backend.rename).toHaveBeenCalledWith('/a.txt', '/archive/a.txt')
  expect(backend.rename).toHaveBeenCalledWith('/b.txt', '/archive/b.txt')
})
