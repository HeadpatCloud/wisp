import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/features/settings/SettingsPage', () => ({
  SettingsPage: () => <div data-testid="settings-page" />,
}))
vi.mock('@/features/transfer/ExportPage', () => ({
  ExportPage: () => <div data-testid="export-page" />,
}))
vi.mock('@/features/transfer/ImportReviewPage', () => ({
  ImportReviewPage: ({ path }: { path: string }) => <div data-testid="import-page">{path}</div>,
}))
vi.mock('./HostsPage', () => ({ HostsPage: () => <div data-testid="hosts-page" /> }))

import type { Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { HostsTab } from './HostsTab'

const host = (id: string, folderId: string | null): Host => ({
  id,
  name: id,
  address: `${id}.lan`,
  folderId,
  color: null,
  order: 0,
  connections: [],
})

beforeEach(() => {
  useHostStore.setState({
    folders: [
      { id: 'work', name: 'Work', parentId: null, order: 0 },
      { id: 'servers', name: 'Servers', parentId: 'work', order: 0 },
    ],
    hosts: [
      host('this-computer', null),
      host('win', 'work'),
      host('build', 'servers'),
      host('db', 'servers'),
      host('loose', null),
    ],
    logins: [],
    keys: [],
    tags: [],
    loaded: true,
  })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
  useSessionStore.setState({ tabs: [], panes: {}, activeTabId: null })
})

test('folder counts include subfolders and All hosts leaves This computer out', () => {
  render(<HostsTab />)
  const nav = screen.getByRole('navigation', { name: 'Main' })
  expect(within(nav).getByRole('button', { name: 'All hosts 4' })).toBeInTheDocument()
  expect(within(nav).getByRole('button', { name: 'Work 3' })).toBeInTheDocument()
  expect(within(nav).getByRole('button', { name: 'Servers 2' })).toBeInTheDocument()
})

test('a folder click filters the page and All hosts clears it', async () => {
  const user = userEvent.setup()
  render(<HostsTab />)
  await user.click(screen.getByRole('button', { name: 'Servers 2' }))
  expect(useHostsUiStore.getState().folderId).toBe('servers')
  await user.click(screen.getByRole('button', { name: 'All hosts 4' }))
  expect(useHostsUiStore.getState().folderId).toBeNull()
})

test('pages open inside the Hosts tab and make it active', () => {
  useSessionStore.setState({ activeTabId: 'some-tab' })
  render(<HostsTab />)
  expect(screen.getByTestId('hosts-page')).toBeInTheDocument()
  act(() => useHostsUiStore.getState().openPage('settings'))
  expect(screen.getByTestId('settings-page')).toBeInTheDocument()
  expect(useSessionStore.getState().activeTabId).toBeNull()
  act(() => useHostsUiStore.getState().openPage('import', 'C:/in.json'))
  expect(screen.getByTestId('import-page')).toHaveTextContent('C:/in.json')
  act(() => useHostsUiStore.getState().openPage('export'))
  expect(screen.getByTestId('export-page')).toBeInTheDocument()
})

test('the Settings entry opens the settings page', async () => {
  const user = userEvent.setup()
  render(<HostsTab />)
  await user.click(screen.getByRole('button', { name: 'Settings' }))
  expect(screen.getByTestId('settings-page')).toBeInTheDocument()
})

test('tags show only when there are any', () => {
  const { rerender } = render(<HostsTab />)
  expect(screen.queryByText('Tags')).not.toBeInTheDocument()
  act(() =>
    useHostStore.setState({
      tags: [
        {
          id: 't',
          name: 'prod',
          color: 'red',
          showColor: true,
          askBeforeClose: true,
          askBeforePaste: true,
          terminalColors: null,
        },
      ],
    }),
  )
  rerender(<HostsTab />)
  expect(screen.getByText('Tags')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'prod' })).toBeInTheDocument()
})
