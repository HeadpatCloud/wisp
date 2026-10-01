import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({}))
vi.mock('@/lib/transfer', () => ({
  readBundle: vi.fn(),
  validateImport: vi.fn().mockResolvedValue([]),
  applyImport: vi.fn().mockResolvedValue({ added: 1, updated: 1 }),
  discardImport: vi.fn().mockResolvedValue(undefined),
}))

import type { ImportReview, ItemProblem } from '@/bindings'
import {
  applyImport,
  discardImport,
  type ReadResult,
  readBundle,
  validateImport,
} from '@/lib/transfer'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import { ImportReviewPage } from './ImportReviewPage'

const review: ImportReview = {
  reviewId: 'r1',
  unchanged: 4,
  items: [
    {
      key: 'ssh:a',
      kind: 'ssh',
      name: 'web',
      status: 'conflict',
      matched: { id: 'pc', name: 'web' },
      fields: [
        { field: 'port', label: 'Port', local: '22', incoming: '2222' },
        { field: 'host', label: 'Host', local: 'a', incoming: 'b' },
      ],
      notes: ['Kept your local keys; not on this machine: /Users/me/id'],
    },
    {
      key: 'ssh:n',
      kind: 'ssh',
      name: 'fresh',
      status: 'new',
      matched: null,
      fields: [],
      notes: [],
    },
  ],
}

const load = vi.fn().mockResolvedValue(undefined)
const removeTab = vi.fn()

function deferred<T>() {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(readBundle).mockReset()
  vi.mocked(validateImport).mockResolvedValue([])
  useProfileStore.setState({ load } as never)
  useSftpProfileStore.setState({ load } as never)
  useS3ProfileStore.setState({ load } as never)
  useSessionStore.setState({ removeTab } as never)
})

test('asks for the password, reports a wrong one, then shows the review', async () => {
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockResolvedValueOnce({ kind: 'wrongPassword' })
    .mockResolvedValueOnce({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await user.type(await screen.findByLabelText('Export password'), 'x')
  await user.click(screen.getByRole('button', { name: 'Open' }))
  expect(await screen.findByText('Wrong password.')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Open' }))
  expect(await screen.findByText(/1 new/)).toBeInTheDocument()
  expect(screen.getByText(/1 with changes/)).toBeInTheDocument()
  expect(screen.getByText(/4 unchanged/)).toBeInTheDocument()
})

test('field toggles, apply payload, reload and summary', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  expect(
    await screen.findByText('Kept your local keys; not on this machine: /Users/me/id'),
  ).toBeInTheDocument()
  await user.click(screen.getByLabelText('Accept Host'))
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(applyImport).toHaveBeenCalledWith('r1', [
    { key: 'ssh:a', accept: true, asNew: false, fields: ['port'] },
    { key: 'ssh:n', accept: true, asNew: false, fields: [] },
  ])
  expect(await screen.findByText('Imported: 1 added, 1 updated.')).toBeInTheDocument()
  expect(load).toHaveBeenCalledTimes(3)
})

test('decline all turns every row off', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await user.click(await screen.findByRole('button', { name: 'Decline all' }))
  expect(screen.getByLabelText('Import web')).not.toBeChecked()
  expect(screen.getByLabelText('Import fresh')).not.toBeChecked()
})

test('accept all after decline all checks every row again', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await user.click(await screen.findByRole('button', { name: 'Decline all' }))
  expect(screen.getByLabelText('Import web')).not.toBeChecked()
  expect(screen.getByRole('button', { name: 'Apply' })).toBeDisabled()
  await user.click(screen.getByRole('button', { name: 'Accept all' }))
  expect(screen.getByLabelText('Import web')).toBeChecked()
  expect(screen.getByLabelText('Import fresh')).toBeChecked()
  expect(screen.getByLabelText('Accept Port')).toBeChecked()
  expect(screen.getByLabelText('Accept Host')).toBeChecked()
  expect(screen.getByRole('button', { name: 'Apply' })).toBeEnabled()
})

test('add as new instead is sent with the apply', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await user.click(await screen.findByLabelText('Add as new instead'))
  expect(screen.getByText('It will be added as a separate profile.')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(applyImport).toHaveBeenCalledWith('r1', [
    { key: 'ssh:a', accept: true, asNew: true, fields: [] },
    { key: 'ssh:n', accept: true, asNew: false, fields: [] },
  ])
})

test('validation problems are shown and block apply', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  vi.mocked(validateImport).mockResolvedValue([
    { key: 'ssh:n', message: 'Key login needs at least one private key' },
  ])
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  expect(await screen.findByText(/Key login needs at least one private key/)).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Apply' })).toBeDisabled()
})

test('a validation answer for outdated decisions is ignored', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const first = deferred<ItemProblem[]>()
  vi.mocked(validateImport).mockReturnValueOnce(first.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await waitFor(() => expect(validateImport).toHaveBeenCalledTimes(1))
  await user.click(screen.getByLabelText('Accept Host'))
  await waitFor(() => expect(validateImport).toHaveBeenCalledTimes(2))
  await act(async () =>
    first.resolve([{ key: 'ssh:n', message: 'Its jump hosts would form a loop' }]),
  )
  expect(screen.queryByText(/Its jump hosts would form a loop/)).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Apply' })).toBeEnabled()
})

test('repeated notes and problems are listed once', async () => {
  const consoleError = vi.spyOn(console, 'error')
  const note = 'Kept your local keys; not on this machine: /Users/me/id'
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: { ...review, items: [{ ...review.items[0], notes: [note, note] }, review.items[1]] },
  })
  const own = { key: 'ssh:a', message: 'Its jump hosts would form a loop' }
  const other = { key: 'ssh:n', message: 'Key login needs at least one private key' }
  vi.mocked(validateImport).mockResolvedValue([own, own, other, other])
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  expect(
    await screen.findAllByText('fresh: Key login needs at least one private key'),
  ).toHaveLength(1)
  expect(screen.getAllByText('Its jump hosts would form a loop')).toHaveLength(1)
  expect(screen.getAllByText(note)).toHaveLength(1)
  expect(consoleError).not.toHaveBeenCalled()
  consoleError.mockRestore()
})

test('discards the pending import on unmount', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const { unmount } = render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await screen.findByText(/1 new/)
  unmount()
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r1'))
})

test('a read that finishes after unmount is discarded', async () => {
  const read = deferred<ReadResult>()
  vi.mocked(readBundle).mockReturnValueOnce(read.promise)
  const { unmount } = render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  unmount()
  expect(discardImport).not.toHaveBeenCalled()
  read.resolve({ kind: 'review', review: { ...review, reviewId: 'r2' } })
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r2'))
})

test('a file that cannot be read shows the message without an Error prefix', async () => {
  vi.mocked(readBundle).mockRejectedValueOnce(new Error('import error: not a wisp export'))
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  expect(await screen.findByText('import error: not a wisp export')).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
})

test('a failed apply shows the message and keeps the review open', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  vi.mocked(applyImport).mockRejectedValueOnce(
    new Error('import error: 1 item(s) still need attention'),
  )
  const user = userEvent.setup()
  const { unmount } = render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  const apply = await screen.findByRole('button', { name: 'Apply' })
  await user.click(apply)
  expect(
    await screen.findByText('import error: 1 item(s) still need attention'),
  ).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
  expect(screen.getByText(/1 new/)).toBeInTheDocument()
  expect(screen.getByLabelText('Import web')).toBeChecked()
  expect(apply).toBeEnabled()
  expect(load).not.toHaveBeenCalled()
  unmount()
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r1'))
})

test('Open is disabled while the password is checked and Enter submits', async () => {
  const read = deferred<ReadResult>()
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockReturnValueOnce(read.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  const open = await screen.findByRole('button', { name: 'Open' })
  expect(open).toBeDisabled()
  await user.type(screen.getByLabelText('Export password'), 'pw')
  expect(open).toBeEnabled()
  await user.keyboard('{Enter}')
  expect(readBundle).toHaveBeenLastCalledWith('C:/in.json', 'pw')
  expect(open).toBeDisabled()
  await user.click(open)
  expect(readBundle).toHaveBeenCalledTimes(2)
  await act(async () => read.resolve({ kind: 'review', review }))
  expect(screen.getByText(/1 new/)).toBeInTheDocument()
})

test('a failed password read shows its message and the next attempt clears it', async () => {
  const retry = deferred<ReadResult>()
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockRejectedValueOnce(new Error('io: C:/in.json: not found'))
    .mockReturnValueOnce(retry.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage tabId="t" path="C:/in.json" />)
  await user.type(await screen.findByLabelText('Export password'), 'pw')
  const open = screen.getByRole('button', { name: 'Open' })
  await user.click(open)
  expect(await screen.findByText('io: C:/in.json: not found')).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
  expect(open).toBeEnabled()
  await user.click(open)
  expect(screen.queryByText('io: C:/in.json: not found')).not.toBeInTheDocument()
  await act(async () => retry.resolve({ kind: 'wrongPassword' }))
  expect(screen.getByText('Wrong password.')).toBeInTheDocument()
  expect(open).toBeEnabled()
})
