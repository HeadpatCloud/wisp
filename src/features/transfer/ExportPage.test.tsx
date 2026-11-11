import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({}))
vi.mock('@/lib/transfer', () => ({
  pickExportPath: vi.fn().mockResolvedValue('C:/out.json'),
  exportBundle: vi.fn().mockResolvedValue({
    hosts: 2,
    connections: 3,
    secrets: 0,
    keyFiles: 0,
    warnings: [],
  }),
}))

import type { Host } from '@/bindings'
import { exportBundle, pickExportPath } from '@/lib/transfer'
import { useHostStore } from '@/stores/hostStore'
import { ExportPage } from './ExportPage'

function host(id: string, name: string, folderId: string | null): Host {
  return { id, name, address: `${name}.lan`, folderId, color: null, order: 0, connections: [] }
}

const folders = [
  { id: 'work', name: 'Work', parentId: null, order: 0 },
  { id: 'servers', name: 'Servers', parentId: 'work', order: 0 },
  { id: 'lab', name: 'Lab', parentId: null, order: 1 },
]

beforeEach(() => {
  vi.clearAllMocks()
  useHostStore.setState({
    folders,
    hosts: [
      host('this-computer', 'This computer', null),
      host('win', 'win-dev', 'work'),
      host('build', 'build-01', 'servers'),
      host('nas', 'nas', 'lab'),
      host('loose', 'loose', null),
    ],
    logins: [],
    keys: [],
    tags: [],
    loaded: true,
  })
})

test('selecting a folder exports it with its subfolders and their hosts', async () => {
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('Work'))
  expect(screen.getByLabelText('Servers')).toBeChecked()
  expect(screen.getByLabelText('build-01')).toBeChecked()
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { folderIds: ['work', 'servers'], hostIds: ['win', 'build'] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
  expect(await screen.findByText(/Exported 2 hosts with 3 connections/)).toBeInTheDocument()
})

test('a single host is exported without its folder being picked', async () => {
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('nas'))
  expect(screen.getByLabelText('Lab')).toBeChecked()
  await user.click(screen.getByLabelText('loose'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(vi.mocked(exportBundle).mock.calls[0][0]).toEqual({
    folderIds: ['lab'],
    hostIds: ['nas', 'loose'],
  })
})

test('This computer is never offered', () => {
  render(<ExportPage onClose={vi.fn()} />)
  expect(screen.queryByLabelText('This computer')).not.toBeInTheDocument()
})

test('nothing selected means nothing to export', () => {
  render(<ExportPage onClose={vi.fn()} />)
  expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled()
})

test('passwords and keys need a matching export password', async () => {
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('nas'))
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton).toBeDisabled()
  await user.type(screen.getByLabelText('Export password'), 'pw')
  await user.type(screen.getByLabelText('Confirm password'), 'pX')
  expect(screen.getByText("Passwords don't match.")).toBeInTheDocument()
  expect(exportButton).toBeDisabled()
  await user.clear(screen.getByLabelText('Confirm password'))
  await user.type(screen.getByLabelText('Confirm password'), 'pw')
  await user.click(exportButton)
  expect(vi.mocked(exportBundle).mock.calls[0].slice(1, 3)).toEqual([
    { includeSecrets: true, includeKeys: false },
    'pw',
  ])
})

test('a failed export is shown and nothing else happens', async () => {
  vi.mocked(exportBundle).mockRejectedValueOnce(new Error('io: denied'))
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('nas'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(await screen.findByText('io: denied')).toBeInTheDocument()
})

test('a cancelled save dialog exports nothing', async () => {
  vi.mocked(pickExportPath).mockResolvedValueOnce(null)
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('nas'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).not.toHaveBeenCalled()
})

test('the summary lists each warning once and uses singular wording for one', async () => {
  vi.mocked(exportBundle).mockResolvedValueOnce({
    hosts: 1,
    connections: 1,
    secrets: 1,
    keyFiles: 1,
    warnings: ["Couldn't read key file C:/k", "Couldn't read key file C:/k"],
  })
  const user = userEvent.setup()
  render(<ExportPage onClose={vi.fn()} />)
  await user.click(screen.getByLabelText('nas'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(
    await screen.findByText('Exported 1 host with 1 connection (1 password, 1 key file).'),
  ).toBeInTheDocument()
  expect(screen.getAllByText("Couldn't read key file C:/k")).toHaveLength(1)
})

test('cancel goes back', async () => {
  const onClose = vi.fn()
  const user = userEvent.setup()
  render(<ExportPage onClose={onClose} />)
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(onClose).toHaveBeenCalled()
})
