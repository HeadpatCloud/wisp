import { beforeEach, expect, test, vi } from 'vitest'

const update = vi.hoisted(() => ({
  version: '0.7.0',
  currentVersion: '0.6.0',
  close: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/updater', () => ({
  checkForUpdate: vi.fn().mockResolvedValue(update),
  isVersionSkipped: () => false,
  skipVersion: vi.fn(),
  downloadUpdate: vi.fn().mockResolvedValue(undefined),
  installUpdate: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/features/sessions/tabHosts', () => ({ hostsAskingToClose: vi.fn() }))
vi.mock('@/stores/confirmStore', () => ({ confirm: vi.fn() }))

import { hostsAskingToClose } from '@/features/sessions/tabHosts'
import { downloadUpdate, installUpdate } from '@/lib/updater'
import { confirm } from '@/stores/confirmStore'
import { useNoticeStore } from '@/stores/noticeStore'
import { startUpdateCheck } from './updateNotice'

beforeEach(() => useNoticeStore.setState({ notices: [] }))

const ready = () =>
  useNoticeStore.getState().notices.find((n) => n.title === 'Wisp 0.7.0 is ready.')
const restartNow = () => {
  const n = ready()
  const action = n?.actions?.find((a) => a.label === 'Restart now')
  if (!n || !action) throw new Error('no ready notice')
  useNoticeStore.getState().act(n.id, action)
}

test('an_update_downloads_quietly_and_restarting_asks_for_tagged_tabs', async () => {
  await startUpdateCheck()
  expect(downloadUpdate).toHaveBeenCalledWith(update, expect.any(Function))
  expect(ready()).toMatchObject({ body: 'It installs when you choose Restart now.', sticky: true })

  // A tab of a tagged host asks first; Cancel installs nothing and keeps the offer.
  vi.mocked(hostsAskingToClose).mockReturnValue('prod-db')
  vi.mocked(confirm).mockResolvedValueOnce(false)
  restartNow()
  await vi.waitFor(() => expect(ready()).toBeDefined())
  expect(confirm).toHaveBeenCalledWith(
    expect.objectContaining({ body: expect.stringContaining('prod-db') }),
  )
  expect(installUpdate).not.toHaveBeenCalled()

  vi.mocked(confirm).mockResolvedValueOnce(true)
  restartNow()
  await vi.waitFor(() => expect(installUpdate).toHaveBeenCalledWith(update))
})
