import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { SettingsPage } from './SettingsPage'

vi.mock('@xterm/xterm', () => ({
  Terminal: vi.fn().mockImplementation(function (this: object) {
    Object.assign(this, {
      open: vi.fn(),
      loadAddon: vi.fn(),
      write: vi.fn(),
      dispose: vi.fn(),
      options: {},
    })
  }),
}))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: vi.fn().mockImplementation(function (this: { fit: () => void }) {
    this.fit = vi.fn()
  }),
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))
vi.mock('@/lib/vault', () => ({ vaultChangePassword: vi.fn().mockResolvedValue(undefined) }))

import { vaultChangePassword } from '@/lib/vault'

const update = vi.fn(async (patch: Record<string, unknown>) => {
  useSettingsStore.setState((st) => ({ settings: { ...st.settings, ...patch } }) as never)
})
const removeTab = vi.fn()

beforeEach(() => {
  vi.clearAllMocks()
  useSettingsStore.setState({
    settings: {
      theme: 'system',
      fontFamily: 'monospace',
      fontSize: 14,
      colorScheme: 'default',
      vncClipboardSync: false,
    },
    loaded: true,
    update,
  } as never)
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null, removeTab } as never)
})

test('changing theme persists via update', async () => {
  const user = userEvent.setup()
  render(<SettingsPage tabId="s1" />)
  await user.click(screen.getByLabelText('Theme'))
  await user.click(await screen.findByText('Dark'))
  expect(update).toHaveBeenCalledWith(expect.objectContaining({ theme: 'dark' }))
})

test('switching to Appearance shows font controls and changing font family calls update', async () => {
  const user = userEvent.setup()
  render(<SettingsPage tabId="s1" />)
  await user.click(screen.getByRole('button', { name: 'Appearance' }))
  fireEvent.change(screen.getByLabelText('Font family'), { target: { value: 'JetBrains Mono' } })
  expect(update).toHaveBeenCalledWith(expect.objectContaining({ fontFamily: 'JetBrains Mono' }))
})

test('changing font size in Appearance section persists via update', async () => {
  const user = userEvent.setup()
  render(<SettingsPage tabId="s1" />)
  await user.click(screen.getByRole('button', { name: 'Appearance' }))
  fireEvent.change(screen.getByLabelText('Font size'), { target: { value: '18' } })
  expect(update).toHaveBeenCalledWith(expect.objectContaining({ fontSize: 18 }))
})

test('Security section sets a master password via vaultChangePassword', async () => {
  const user = userEvent.setup()
  render(<SettingsPage tabId="s1" />)
  await user.click(screen.getByRole('button', { name: 'Security' }))
  await user.type(screen.getByLabelText('Master password'), 'sup3r-secret')
  await user.click(screen.getByRole('button', { name: /set master password/i }))
  expect(vaultChangePassword).toHaveBeenCalledWith('sup3r-secret')
})

test('close button calls removeTab', async () => {
  const user = userEvent.setup()
  render(<SettingsPage tabId="s1" />)
  await user.click(screen.getByRole('button', { name: 'Close' }))
  expect(removeTab).toHaveBeenCalledWith('s1')
})
