import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { StrictMode } from 'react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/bindings', () => ({ commands: {} }))
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
import { useHostStore } from '@/stores/hostStore'
import { ImportReviewPage } from './ImportReviewPage'

const review: ImportReview = {
  reviewId: 'r1',
  unchanged: 4,
  items: [
    {
      key: 'ssh:a',
      kind: 'connection',
      parent: null,
      name: 'web',
      status: 'conflict',
      matched: { id: 'pc', name: 'web' },
      fields: [
        { field: 'port', label: 'Port', local: '22', incoming: '2222' },
        { field: 'host', label: 'Host', local: 'a', incoming: 'b' },
      ],
      notes: ['Kept your local keys; not on this machine: /Users/me/id'],
      notesAsNew: ['Key file not on this machine: /Users/me/id'],
    },
    {
      key: 'ssh:n',
      kind: 'connection',
      parent: null,
      name: 'fresh',
      status: 'new',
      matched: null,
      fields: [],
      notes: [],
      notesAsNew: [],
    },
  ],
}

const load = vi.fn().mockResolvedValue(undefined)
const onClose = vi.fn()

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
  useHostStore.setState({ load } as never)
})

afterEach(() => {
  vi.restoreAllMocks()
})

test('asks for the password, reports a wrong one, then shows the review', async () => {
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockResolvedValueOnce({ kind: 'wrongPassword' })
    .mockResolvedValueOnce({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
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
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
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
  expect(load).toHaveBeenCalledTimes(1)
})

test('a VNC profile is labelled VNC in the list and in its details', async () => {
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: {
      ...review,
      items: [
        {
          key: 'vnc:m',
          kind: 'connection',
          parent: null,
          name: 'desk',
          status: 'conflict',
          matched: { id: 'pc', name: 'tower' },
          fields: [
            { field: 'password', label: 'Password', local: '••••', incoming: '•••• (different)' },
          ],
          notes: [],
          notesAsNew: [],
        },
        { ...review.items[1], key: 'vnc:n', kind: 'connection', parent: null, name: 'lab' },
      ],
    },
  })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText('Matches your Connection "tower"')).toBeInTheDocument()
  expect(screen.getAllByText('Connection')).toHaveLength(2)
  await user.click(screen.getByRole('button', { name: 'lab' }))
  expect(screen.getByText('New Connection')).toBeInTheDocument()
})

test('an RDP profile is labelled RDP in the list and in its details', async () => {
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: {
      ...review,
      items: [
        {
          key: 'rdp:m',
          kind: 'connection',
          parent: null,
          name: 'office',
          status: 'conflict',
          matched: { id: 'pc', name: 'tower' },
          fields: [
            {
              field: 'keyboardLayout',
              label: 'Keyboard layout',
              local: 'Automatic',
              incoming: 'German',
            },
          ],
          notes: [],
          notesAsNew: [],
        },
        { ...review.items[1], key: 'rdp:n', kind: 'connection', parent: null, name: 'lab' },
      ],
    },
  })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText('Matches your Connection "tower"')).toBeInTheDocument()
  expect(screen.getAllByText('Connection')).toHaveLength(2)
  expect(screen.getByText('German')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'lab' }))
  expect(screen.getByText('New Connection')).toBeInTheDocument()
})

test('RDP sound, shared folder and gateway rows show with the shared folder notes', async () => {
  const kept = 'Kept your local shared folder; not on this machine: /home/faye/share'
  const leftOut = 'Shared folder left out; not on this machine: /home/faye/share'
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: {
      ...review,
      items: [
        {
          key: 'rdp:m',
          kind: 'connection',
          parent: null,
          name: 'office',
          status: 'conflict',
          matched: { id: 'pc', name: 'office' },
          fields: [
            { field: 'audio', label: 'Sound', local: 'On', incoming: 'Off' },
            {
              field: 'gateway',
              label: 'RD Gateway',
              local: 'gw.corp:443, same login',
              incoming: 'gw.corp:443, CORP\\gw',
            },
            {
              field: 'gatewayPassword',
              label: 'Gateway password',
              local: '(none)',
              incoming: '••••',
            },
          ],
          notes: [kept],
          notesAsNew: [leftOut],
        },
        {
          key: 'rdp:t',
          kind: 'connection',
          parent: null,
          name: 'tower',
          status: 'conflict',
          matched: { id: 't', name: 'tower' },
          fields: [
            {
              field: 'sharedFolder',
              label: 'Shared folder',
              local: 'None',
              incoming: 'C:\\Users\\faye as faye',
            },
          ],
          notes: [],
          notesAsNew: [],
        },
      ],
    },
  })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText('Sound')).toBeInTheDocument()
  expect(screen.getByText('On')).toBeInTheDocument()
  expect(screen.getByText('Off')).toBeInTheDocument()
  expect(screen.getByText('RD Gateway')).toBeInTheDocument()
  expect(screen.getByText('gw.corp:443, same login')).toBeInTheDocument()
  expect(screen.getByText('gw.corp:443, CORP\\gw')).toBeInTheDocument()
  expect(screen.getByText('Gateway password')).toBeInTheDocument()
  expect(screen.getByText('••••')).toBeInTheDocument()
  expect(screen.getByText(kept)).toBeInTheDocument()
  expect(screen.queryByText(leftOut)).not.toBeInTheDocument()

  await user.click(screen.getByLabelText('Add as new instead'))
  expect(screen.getByText(leftOut)).toBeInTheDocument()
  expect(screen.queryByText(kept)).not.toBeInTheDocument()
  await user.click(screen.getByLabelText('Add as new instead'))
  await user.click(screen.getByLabelText('Accept Gateway password'))

  await user.click(screen.getByRole('button', { name: 'tower' }))
  expect(screen.getByText('Shared folder')).toBeInTheDocument()
  expect(screen.getByText('C:\\Users\\faye as faye')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(applyImport).toHaveBeenCalledWith('r1', [
    { key: 'rdp:m', accept: true, asNew: false, fields: ['audio', 'gateway'] },
    { key: 'rdp:t', accept: true, asNew: false, fields: ['sharedFolder'] },
  ])
})

test('the first changed profile is selected even when a new one comes first', async () => {
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: { ...review, items: [review.items[1], review.items[0]] },
  })
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText('Matches your Connection "web"')).toBeInTheDocument()
  expect(screen.getByLabelText('Accept Host')).toBeInTheDocument()
  expect(screen.queryByText('New Connection')).not.toBeInTheDocument()
})

test('decline all turns every row off', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await user.click(await screen.findByRole('button', { name: 'Decline all' }))
  expect(screen.getByLabelText('Import web')).not.toBeChecked()
  expect(screen.getByLabelText('Import fresh')).not.toBeChecked()
})

test('accept all after decline all checks every row again', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
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
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await user.click(await screen.findByLabelText('Add as new instead'))
  expect(screen.getByText('It will be added separately.')).toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(applyImport).toHaveBeenCalledWith('r1', [
    { key: 'ssh:a', accept: true, asNew: true, fields: [] },
    { key: 'ssh:n', accept: true, asNew: false, fields: [] },
  ])
})

test('add as new instead shows the notes a new profile gets', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const kept = 'Kept your local keys; not on this machine: /Users/me/id'
  const asNew = 'Key file not on this machine: /Users/me/id'
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  const toggle = await screen.findByLabelText('Add as new instead')
  expect(screen.getByText(kept)).toBeInTheDocument()
  expect(screen.queryByText(asNew)).not.toBeInTheDocument()
  await user.click(toggle)
  expect(screen.getByText(asNew)).toBeInTheDocument()
  expect(screen.queryByText(kept)).not.toBeInTheDocument()
  await user.click(toggle)
  expect(screen.getByText(kept)).toBeInTheDocument()
  expect(screen.queryByText(asNew)).not.toBeInTheDocument()
})

test('validation problems are shown and block apply', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  vi.mocked(validateImport).mockResolvedValue([
    { key: 'ssh:n', message: 'Key login needs at least one private key' },
  ])
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText(/Key login needs at least one private key/)).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Apply' })).toBeDisabled()
})

test('a validation answer for outdated decisions is ignored', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const first = deferred<ItemProblem[]>()
  vi.mocked(validateImport).mockReturnValueOnce(first.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
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
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(
    await screen.findAllByText('fresh: Key login needs at least one private key'),
  ).toHaveLength(1)
  expect(screen.getAllByText('Its jump hosts would form a loop')).toHaveLength(1)
  expect(screen.getAllByText(note)).toHaveLength(1)
  expect(consoleError).not.toHaveBeenCalled()
})

test('a decision changed right before Apply is not validated after the apply', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await waitFor(() => expect(validateImport).toHaveBeenCalledTimes(1))
  vi.mocked(validateImport).mockRejectedValue(new Error('not found: import r1'))
  await user.click(screen.getByLabelText('Accept Host'))
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(await screen.findByText('Imported: 1 added, 1 updated.')).toBeInTheDocument()
  await new Promise((resolve) => setTimeout(resolve, 300))
  expect(validateImport).toHaveBeenCalledTimes(1)
  expect(screen.queryByText(/not found/)).not.toBeInTheDocument()
})

test('discards the pending import on unmount', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  const { unmount } = render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await screen.findByText(/1 new/)
  unmount()
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r1'))
})

test('a read that finishes after unmount is discarded', async () => {
  const read = deferred<ReadResult>()
  vi.mocked(readBundle).mockReturnValueOnce(read.promise)
  const { unmount } = render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  unmount()
  expect(discardImport).not.toHaveBeenCalled()
  read.resolve({ kind: 'review', review: { ...review, reviewId: 'r2' } })
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r2'))
})

test('under StrictMode every review the page received is shown or discarded', async () => {
  const stale = { ...review, reviewId: 'first', items: [{ ...review.items[1], name: 'stale' }] }
  const shown = { ...review, reviewId: 'second', items: [{ ...review.items[1], name: 'shown' }] }
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'review', review: stale })
    .mockResolvedValueOnce({ kind: 'review', review: shown })
  const { unmount } = render(
    <StrictMode>
      <ImportReviewPage path="C:/in.json" onClose={onClose} />
    </StrictMode>,
  )
  expect(await screen.findByLabelText('Import shown')).toBeInTheDocument()
  expect(screen.queryByLabelText('Import stale')).not.toBeInTheDocument()
  expect(readBundle).toHaveBeenCalledTimes(2)
  expect(vi.mocked(discardImport).mock.calls).toEqual([['first']])
  unmount()
  expect(vi.mocked(discardImport).mock.calls).toEqual([['first'], ['second']])
})

test('a file that cannot be read shows the message without an Error prefix', async () => {
  vi.mocked(readBundle).mockRejectedValueOnce(new Error('import error: not a wisp export'))
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  expect(await screen.findByText('import error: not a wisp export')).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
})

test('a failed apply shows the message and keeps the review open', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  vi.mocked(applyImport).mockRejectedValueOnce(new Error('io: denied'))
  const user = userEvent.setup()
  const { unmount } = render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await waitFor(() => expect(validateImport).toHaveBeenCalledTimes(1))
  const apply = screen.getByRole('button', { name: 'Apply' })
  await user.click(apply)
  expect(await screen.findByText('io: denied')).toBeInTheDocument()
  expect(screen.queryByText(/Error:/)).not.toBeInTheDocument()
  expect(screen.getByText(/1 new/)).toBeInTheDocument()
  expect(screen.getByLabelText('Import web')).toBeChecked()
  expect(apply).toBeEnabled()
  expect(load).not.toHaveBeenCalled()
  await waitFor(() => expect(validateImport).toHaveBeenCalledTimes(2))
  await new Promise((resolve) => setTimeout(resolve, 50))
  expect(screen.getByText('io: denied')).toBeInTheDocument()
  unmount()
  await waitFor(() => expect(discardImport).toHaveBeenCalledWith('r1'))
})

test('a failed apply refreshes the problems and its message goes once they are fixed', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  vi.mocked(applyImport).mockRejectedValueOnce(
    new Error('import error: 1 item(s) still need attention'),
  )
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  const apply = await screen.findByRole('button', { name: 'Apply' })
  vi.mocked(validateImport).mockResolvedValue([
    { key: 'ssh:n', message: 'Key login needs at least one private key' },
  ])
  await user.click(apply)
  expect(
    await screen.findByText('import error: 1 item(s) still need attention'),
  ).toBeInTheDocument()
  expect(
    await screen.findByText('fresh: Key login needs at least one private key'),
  ).toBeInTheDocument()
  expect(apply).toBeDisabled()
  vi.mocked(validateImport).mockResolvedValue([])
  await user.click(screen.getByLabelText('Import fresh'))
  await waitFor(() => expect(screen.queryByText(/Key login needs/)).not.toBeInTheDocument())
  expect(screen.queryByText(/still need attention/)).not.toBeInTheDocument()
  expect(apply).toBeEnabled()
})

test('a failed host list reload still shows the import summary', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'review', review })
  load.mockRejectedValueOnce(new Error('io: denied'))
  const user = userEvent.setup()
  const { unmount } = render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await user.click(await screen.findByRole('button', { name: 'Apply' }))
  expect(await screen.findByText('Imported: 1 added, 1 updated.')).toBeInTheDocument()
  expect(
    await screen.findByText('The host list could not be refreshed: io: denied'),
  ).toBeInTheDocument()
  expect(load).toHaveBeenCalledTimes(1)
  expect(screen.queryByRole('button', { name: 'Apply' })).not.toBeInTheDocument()
  unmount()
  expect(discardImport).not.toHaveBeenCalled()
})

test('Open is disabled while the password is checked and Enter submits', async () => {
  const read = deferred<ReadResult>()
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockReturnValueOnce(read.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  const open = await screen.findByRole('button', { name: 'Open' })
  expect(open).toBeDisabled()
  await user.type(screen.getByLabelText('Export password'), 'pw')
  expect(open).toBeEnabled()
  await user.keyboard('{Enter}')
  expect(readBundle).toHaveBeenLastCalledWith('C:/in.json', 'pw')
  expect(open).toBeDisabled()
  await user.keyboard('{Enter}')
  expect(readBundle).toHaveBeenCalledTimes(2)
  await act(async () => read.resolve({ kind: 'review', review }))
  expect(screen.getByText(/1 new/)).toBeInTheDocument()
})

test('Open submits its own tab when another import tab also asks for a password', async () => {
  vi.mocked(readBundle).mockResolvedValue({ kind: 'needsPassword' })
  const user = userEvent.setup()
  render(
    <>
      <div className="hidden">
        <ImportReviewPage path="C:/a.json" onClose={onClose} />
      </div>
      <div data-testid="second">
        <ImportReviewPage path="C:/b.json" onClose={onClose} />
      </div>
    </>,
  )
  const second = within(screen.getByTestId('second'))
  await user.type(await second.findByLabelText('Export password'), 'pw')
  await user.click(second.getByRole('button', { name: 'Open' }))
  expect(readBundle).toHaveBeenCalledWith('C:/b.json', 'pw')
  expect(readBundle).not.toHaveBeenCalledWith('C:/a.json', expect.any(String))
})

test('a failed password read shows its message and the next attempt clears it', async () => {
  const retry = deferred<ReadResult>()
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockRejectedValueOnce(new Error('io: C:/in.json: not found'))
    .mockReturnValueOnce(retry.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
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

test('Wrong password. goes away while the next attempt is checked', async () => {
  const retry = deferred<ReadResult>()
  vi.mocked(readBundle)
    .mockResolvedValueOnce({ kind: 'needsPassword' })
    .mockResolvedValueOnce({ kind: 'wrongPassword' })
    .mockReturnValueOnce(retry.promise)
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  await user.type(await screen.findByLabelText('Export password'), 'x')
  const open = screen.getByRole('button', { name: 'Open' })
  await user.click(open)
  expect(await screen.findByText('Wrong password.')).toBeInTheDocument()
  await user.click(open)
  expect(screen.queryByText('Wrong password.')).not.toBeInTheDocument()
  await act(async () => retry.resolve({ kind: 'review', review }))
  expect(screen.getByText(/1 new/)).toBeInTheDocument()
})

test('a new host lists its connections under it, and declining it declines them', async () => {
  const item = review.items[1]
  vi.mocked(readBundle).mockResolvedValue({
    kind: 'review',
    review: {
      ...review,
      items: [
        { ...item, key: 'host:h', kind: 'host', name: 'nas' },
        { ...item, key: 'conn:a', kind: 'connection', name: 'nas · SSH · admin', parent: 'host:h' },
      ],
    },
  })
  const user = userEvent.setup()
  render(<ImportReviewPage path="C:/in.json" onClose={onClose} />)
  const host = await screen.findByRole('button', { name: 'nas' })
  const connection = screen.getByRole('button', { name: 'nas · SSH · admin' })
  const nested = connection.closest('ul')?.parentElement
  expect(nested?.tagName).toBe('LI')
  expect(nested?.previousElementSibling).toContainElement(host)
  await user.click(screen.getByLabelText('Import nas'))
  await user.click(screen.getByRole('button', { name: 'Apply' }))
  expect(applyImport).not.toHaveBeenCalled()
})
