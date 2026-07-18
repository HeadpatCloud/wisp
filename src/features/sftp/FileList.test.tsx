import { render, screen, within } from '@testing-library/react'
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
      selected={new Set<string>()}
      sort={{ key: 'name', desc: false }}
      onSort={vi.fn()}
      onRowClick={vi.fn()}
      onEnter={vi.fn()}
      onDownload={vi.fn()}
      onRename={vi.fn()}
      onDelete={vi.fn()}
      onCopyPath={vi.fn()}
      onCopyName={vi.fn()}
      onEdit={vi.fn()}
      renaming={null}
      onRenameSubmit={vi.fn()}
      onRenameCancel={vi.fn()}
      keyNav={{ current: false }}
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

test('right-clicking an unselected row selects it and names it in the menu', async () => {
  const onRowClick = vi.fn()
  const user = userEvent.setup()
  renderList({ onRowClick })
  await user.pointer({ target: screen.getByText('a.txt'), keys: '[MouseRight]' })
  expect(onRowClick).toHaveBeenCalledWith(expect.objectContaining({ path: '/a.txt' }), {
    toggle: false,
    range: false,
  })
  expect(within(screen.getByRole('menu')).getByText('a.txt')).toBeInTheDocument()
})

test('right-clicking inside a multi-selection keeps it and shows the count', async () => {
  const onRowClick = vi.fn()
  const user = userEvent.setup()
  renderList({ onRowClick, selected: new Set(['/docs', '/a.txt']) })
  await user.pointer({ target: screen.getByText('a.txt'), keys: '[MouseRight]' })
  expect(onRowClick).not.toHaveBeenCalled()
  expect(within(screen.getByRole('menu')).getByText('2 items')).toBeInTheDocument()
})

test('folders and files get different icons', () => {
  renderList()
  const iconOf = (name: string) =>
    screen.getByText(name).closest('li')?.querySelector('svg')?.getAttribute('class')
  expect(iconOf('docs')).toContain('text-primary')
  expect(iconOf('a.txt')).toContain('text-muted-foreground')
})

test('link actions appear only when the backend supplies them', async () => {
  const user = userEvent.setup()
  renderList()
  await user.pointer({ target: screen.getByText('a.txt'), keys: '[MouseRight]' })
  expect(within(screen.getByRole('menu')).queryByText('Copy URL')).not.toBeInTheDocument()
  expect(within(screen.getByRole('menu')).queryByText('Shareable link…')).not.toBeInTheDocument()
})

test('shareable link is offered for files but not folders', async () => {
  const onSignedLink = vi.fn()
  const user = userEvent.setup()
  renderList({ onCopyUrl: vi.fn(), onSignedLink })
  await user.pointer({ target: screen.getByText('a.txt'), keys: '[MouseRight]' })
  const menu = within(screen.getByRole('menu'))
  expect(menu.getByText('Copy URL')).toBeInTheDocument()
  await user.click(menu.getByText('Shareable link…'))
  expect(onSignedLink).toHaveBeenCalledWith(expect.objectContaining({ path: '/a.txt' }))

  await user.pointer({ target: screen.getByText('docs'), keys: '[MouseRight]' })
  expect(within(screen.getByRole('menu')).queryByText('Shareable link…')).not.toBeInTheDocument()
})

test('clicking a column header sorts by it', async () => {
  const onSort = vi.fn()
  const user = userEvent.setup()
  renderList({ onSort })
  await user.click(screen.getByRole('button', { name: 'Size' }))
  expect(onSort).toHaveBeenCalledWith('size')
})
