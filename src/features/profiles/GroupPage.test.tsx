import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { GroupPage } from './GroupPage'

const saveGroup = vi.fn().mockResolvedValue(undefined)
const removeTab = vi.fn()

beforeEach(() => {
  vi.clearAllMocks()
  useProfileStore.setState({ groups: [], profiles: [], loaded: true, saveGroup } as never)
  useSessionStore.setState({ tabs: [], sessions: {}, activeTabId: null, removeTab } as never)
})

test('saves a group with typed name and closes the tab', async () => {
  const user = userEvent.setup()
  render(<GroupPage groupId={null} tabId="g1" />)
  await user.type(screen.getByLabelText('Name'), 'Prod')
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveGroup).toHaveBeenCalledWith(expect.objectContaining({ name: 'Prod' }))
  expect(removeTab).toHaveBeenCalledWith('g1')
})

test('cancel closes the tab without saving', async () => {
  const user = userEvent.setup()
  render(<GroupPage groupId={null} tabId="g1" />)
  await user.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(removeTab).toHaveBeenCalledWith('g1')
  expect(saveGroup).not.toHaveBeenCalled()
})

test('saves with chosen icon', async () => {
  const user = userEvent.setup()
  render(<GroupPage groupId={null} tabId="g1" />)
  await user.type(screen.getByLabelText('Name'), 'Dev')
  await user.click(screen.getByRole('button', { name: 'icon database' }))
  await user.click(screen.getByRole('button', { name: 'Save' }))
  expect(saveGroup).toHaveBeenCalledWith(
    expect.objectContaining({ name: 'Dev', icon: { kind: 'builtin', name: 'database' } }),
  )
})
