import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, test, vi } from 'vitest'
import { FileList } from './FileList'

const entries = [
  { name: 'docs', path: '/docs', isDir: true, isSymlink: false, size: 0, modified: null },
  { name: 'a.txt', path: '/a.txt', isDir: false, isSymlink: false, size: 2048, modified: null },
] as never[]

function renderList(overrides = {}) {
  return render(
    <FileList
      entries={entries}
      onEnter={vi.fn()}
      onDownload={vi.fn()}
      onRename={vi.fn()}
      onDelete={vi.fn()}
      {...overrides}
    />,
  )
}

test('renders names and sizes; double-click dir enters, file downloads', async () => {
  const onEnter = vi.fn()
  const onDownload = vi.fn()
  const user = userEvent.setup()
  renderList({ onEnter, onDownload })
  expect(screen.getByText('docs')).toBeInTheDocument()
  expect(screen.getByText('2.0 KB')).toBeInTheDocument()
  await user.dblClick(screen.getByText('docs'))
  expect(onEnter).toHaveBeenCalledWith(expect.objectContaining({ path: '/docs' }))
  await user.dblClick(screen.getByText('a.txt'))
  expect(onDownload).toHaveBeenCalledWith(expect.objectContaining({ path: '/a.txt' }))
})

test('context menu Rename calls onRename', async () => {
  const onRename = vi.fn()
  const user = userEvent.setup()
  renderList({ onRename })
  await user.pointer({ target: screen.getByText('a.txt'), keys: '[MouseRight]' })
  await user.click(screen.getByText('Rename'))
  expect(onRename).toHaveBeenCalledWith(expect.objectContaining({ path: '/a.txt' }))
})

test('context menu Delete calls onDelete', async () => {
  const onDelete = vi.fn()
  const user = userEvent.setup()
  renderList({ onDelete })
  await user.pointer({ target: screen.getByText('docs'), keys: '[MouseRight]' })
  await user.click(screen.getByText('Delete'))
  expect(onDelete).toHaveBeenCalledWith(expect.objectContaining({ path: '/docs' }))
})
