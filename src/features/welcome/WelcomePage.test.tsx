import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useSessionStore } from '@/stores/sessionStore'
import { WelcomePage } from './WelcomePage'

const openView = vi.fn()

beforeEach(() => {
  vi.clearAllMocks()
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null, openView } as never)
})

test('New SSH profile button calls openView with profile-editor/null', async () => {
  const user = userEvent.setup()
  render(<WelcomePage />)
  await user.click(screen.getByRole('button', { name: /new ssh profile/i }))
  expect(openView).toHaveBeenCalledWith({ kind: 'profile-editor', profileId: null }, 'New profile')
})

test('Import button calls openView with import', async () => {
  const user = userEvent.setup()
  render(<WelcomePage />)
  await user.click(screen.getByRole('button', { name: /import/i }))
  expect(openView).toHaveBeenCalledWith({ kind: 'import' }, 'Import')
})

test('Settings button calls openView with settings', async () => {
  const user = userEvent.setup()
  render(<WelcomePage />)
  await user.click(screen.getByRole('button', { name: /settings/i }))
  expect(openView).toHaveBeenCalledWith({ kind: 'settings' }, 'Settings')
})
