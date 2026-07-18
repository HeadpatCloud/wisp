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
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@/lib/local', () => ({
  editTempPath: vi.fn().mockResolvedValue('/tmp/wisp-edit/abc/a.txt'),
  fileMtime: vi.fn().mockResolvedValue(100),
}))

import { openPath } from '@tauri-apps/plugin-opener'
import { fileMtime } from '@/lib/local'
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

test('mkdir inside a trailing-slash directory does not double the separator', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  backend.list = vi
    .fn()
    .mockResolvedValue([
      { name: 'sub', path: '/bucket/sub/', isDir: true, isSymlink: false, size: 0, modified: null },
    ])
  render(<FileBrowser backend={backend} initialPath="/bucket" />)
  await user.dblClick(await screen.findByText('sub'))
  await user.click(screen.getByRole('button', { name: 'New folder' }))
  await user.type(await screen.findByRole('textbox'), 'created')
  await user.click(screen.getByRole('button', { name: 'OK' }))
  await waitFor(() => expect(backend.mkdir).toHaveBeenCalledWith('/bucket/sub/created'))
})

// Full round trip: download to temp, hand to the OS editor, then push a save back.
test('edit locally re-uploads to the original directory when the file changes', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  backend.list = vi.fn().mockResolvedValue([
    {
      name: 'a.txt',
      path: '/dir/a.txt',
      isDir: false,
      isSymlink: false,
      size: 10,
      modified: null,
    },
  ])
  vi.mocked(fileMtime).mockResolvedValue(100)
  render(<FileBrowser backend={backend} initialPath="/dir" />)

  await user.pointer({ target: await screen.findByText('a.txt'), keys: '[MouseRight]' })
  await user.click(screen.getByText('Edit locally…'))

  await waitFor(() =>
    expect(backend.downloadTo).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ path: '/dir/a.txt' }),
      '/tmp/wisp-edit/abc/a.txt',
      expect.any(Function),
    ),
  )
  await waitFor(() => expect(openPath).toHaveBeenCalledWith('/tmp/wisp-edit/abc/a.txt'))
  expect(backend.upload).not.toHaveBeenCalled()

  vi.mocked(fileMtime).mockResolvedValue(200)
  await waitFor(
    () =>
      expect(backend.upload).toHaveBeenCalledWith(
        expect.any(String),
        '/tmp/wisp-edit/abc/a.txt',
        '/dir',
        expect.any(Function),
      ),
    { timeout: 4000 },
  )
})

test('hides dotfiles until the toggle is on', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  backend.list = vi.fn().mockResolvedValue([...files, entry('.env', false, 5)])
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  expect(screen.queryByText('.env')).not.toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Show hidden files' }))
  expect(screen.getByText('.env')).toBeInTheDocument()
})

test('hiding dotfiles drops them from the selection', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  backend.list = vi.fn().mockResolvedValue([...files, entry('.env', false, 5)])
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.click(screen.getByRole('button', { name: 'Show hidden files' }))
  await user.keyboard('{Control>}a{/Control}')
  expect(screen.getByText('4 selected · 65 B')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Hide hidden files' }))
  expect(screen.getByText('3 selected · 60 B')).toBeInTheDocument()
})

test('arrow keys move the selection and Backspace navigates up', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  render(<FileBrowser backend={backend} initialPath="/one/two" />)
  await screen.findByText('a.txt')

  await user.keyboard('{ArrowDown}')
  expect(screen.getByText('1 selected · 10 B')).toBeInTheDocument()
  await user.keyboard('{ArrowDown}')
  expect(screen.getByText('1 selected · 20 B')).toBeInTheDocument()
  await user.keyboard('{ArrowUp}')
  expect(screen.getByText('1 selected · 10 B')).toBeInTheDocument()

  await user.keyboard('{Backspace}')
  await waitFor(() => expect(backend.list).toHaveBeenCalledWith('/one'))
})

test('F2 renames inline and commits on Enter', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  render(<FileBrowser backend={backend} />)
  await user.click(await screen.findByText('a.txt'))
  await user.keyboard('{F2}')
  const input = screen.getByDisplayValue('a.txt')
  await user.clear(input)
  await user.type(input, 'renamed.txt{Enter}')
  await waitFor(() => expect(backend.rename).toHaveBeenCalledWith('/a.txt', '/renamed.txt'))
})

test('shows a running count while a bulk delete is in flight', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  let releaseFirst = () => {}
  const gate = new Promise<void>((res) => {
    releaseFirst = res
  })
  backend.remove = vi.fn().mockReturnValueOnce(gate).mockResolvedValue(undefined)
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  await user.click(screen.getByRole('button', { name: 'Delete' }))
  await user.click(screen.getAllByRole('button', { name: 'Delete' }).at(-1) as HTMLElement)
  expect(await screen.findByText('Deleting 0 of 3…')).toBeInTheDocument()
  releaseFirst()
  await waitFor(() => expect(backend.remove).toHaveBeenCalledTimes(3))
})

test('cancel stops a bulk delete at the next item boundary', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  let release = () => {}
  const gate = new Promise<void>((res) => {
    release = res
  })
  backend.remove = vi.fn().mockReturnValueOnce(gate).mockResolvedValue(undefined)
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  await user.click(screen.getByRole('button', { name: 'Delete' }))
  await user.click(screen.getAllByRole('button', { name: 'Delete' }).at(-1) as HTMLElement)
  await screen.findByText('Deleting 0 of 3…')
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  release()
  expect(await screen.findByText('Stopped at 1 of 3')).toBeInTheDocument()
  expect(backend.remove).toHaveBeenCalledTimes(1)
})

test('cancelling while the last item completes still reports success and closes', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  let release = () => {}
  const gate = new Promise<void>((res) => {
    release = res
  })
  backend.remove = vi
    .fn()
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce(undefined)
    .mockReturnValueOnce(gate)
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  await user.click(screen.getByRole('button', { name: 'Delete' }))
  await user.click(screen.getAllByRole('button', { name: 'Delete' }).at(-1) as HTMLElement)
  await waitFor(() => expect(backend.remove).toHaveBeenCalledTimes(3))
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  release()
  await waitFor(() => expect(screen.queryByText(/of 3/)).not.toBeInTheDocument())
  expect(screen.queryByText(/Stopped/)).not.toBeInTheDocument()
})

test('keeps the dialog open with a failure summary when items fail', async () => {
  const user = userEvent.setup()
  const backend = makeBackend()
  backend.remove = vi
    .fn()
    .mockResolvedValueOnce(undefined)
    .mockRejectedValueOnce(new Error('denied'))
    .mockResolvedValueOnce(undefined)
  render(<FileBrowser backend={backend} />)
  await screen.findByText('a.txt')
  await user.keyboard('{Control>}a{/Control}')
  await user.click(screen.getByRole('button', { name: 'Delete' }))
  await user.click(screen.getAllByRole('button', { name: 'Delete' }).at(-1) as HTMLElement)
  expect(await screen.findByText('1 of 3 failed')).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument()
})
