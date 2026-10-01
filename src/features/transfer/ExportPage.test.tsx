import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({}))
vi.mock('@/lib/transfer', () => ({
  pickExportPath: vi.fn().mockResolvedValue('C:/out.json'),
  exportBundle: vi.fn().mockResolvedValue({
    profiles: 2,
    secrets: 1,
    keyFiles: 1,
    warnings: ['Also exported jump host "b"'],
  }),
}))

import { exportBundle } from '@/lib/transfer'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import { ExportPage } from './ExportPage'

const base = {
  host: 'h',
  port: 22,
  username: 'u',
  authMethod: 'password',
  keyPath: null,
  keys: [],
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
  jumpHostId: null,
  tunnels: [],
}

beforeEach(() => {
  vi.clearAllMocks()
  useProfileStore.setState({
    groups: [
      {
        id: 'g',
        name: 'Prod',
        parentId: null,
        icon: { kind: 'builtin', name: 'folder' },
        order: 0,
      },
    ],
    profiles: [
      { ...base, id: 'a', name: 'web', groupId: 'g' },
      { ...base, id: 'b', name: 'db', groupId: null },
    ],
  } as never)
  useSftpProfileStore.setState({ profiles: [] } as never)
  useS3ProfileStore.setState({ profiles: [] } as never)
  useSessionStore.setState({ removeTab: vi.fn() } as never)
})

test('group checkbox selects its profiles and exports without a password', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('Prod'))
  await user.click(screen.getByRole('button', { name: 'Export' }))
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: ['g'], profileIds: ['a'], sftpIds: [], s3Ids: [] },
    { includeSecrets: false, includeKeys: false },
    null,
    'C:/out.json',
  )
  expect(await screen.findByText(/Exported 2 profiles/)).toBeInTheDocument()
  expect(screen.getByText('Also exported jump host "b"')).toBeInTheDocument()
})

test('including secrets requires a matching password', async () => {
  const user = userEvent.setup()
  render(<ExportPage tabId="t" />)
  await user.click(screen.getByLabelText('db'))
  await user.click(screen.getByLabelText('Include passwords and passphrases'))
  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton).toBeDisabled()
  await user.type(screen.getByLabelText('Export password'), 'pw')
  await user.type(screen.getByLabelText('Confirm password'), 'pX')
  expect(exportButton).toBeDisabled()
  await user.clear(screen.getByLabelText('Confirm password'))
  await user.type(screen.getByLabelText('Confirm password'), 'pw')
  await user.click(exportButton)
  expect(exportBundle).toHaveBeenCalledWith(
    { groupIds: [], profileIds: ['b'], sftpIds: [], s3Ids: [] },
    { includeSecrets: true, includeKeys: false },
    'pw',
    'C:/out.json',
  )
})
