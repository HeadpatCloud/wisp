import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { Notices } from '@/components/Notices'
import { notify, useNoticeStore } from './noticeStore'

beforeEach(() => {
  vi.useFakeTimers()
  useNoticeStore.setState({ notices: [] })
})

afterEach(() => {
  vi.useRealTimers()
})

test('a notice goes after 8 seconds and runs onClose', () => {
  const onClose = vi.fn()
  act(() => {
    notify({ title: 'nas.local deleted', onClose })
  })
  act(() => {
    vi.advanceTimersByTime(7999)
  })
  expect(useNoticeStore.getState().notices).toHaveLength(1)
  act(() => {
    vi.advanceTimersByTime(1)
  })
  expect(useNoticeStore.getState().notices).toHaveLength(0)
  expect(onClose).toHaveBeenCalledTimes(1)
})

test('hovering holds a notice', () => {
  render(<Notices />)
  act(() => {
    notify({ title: 'held' })
  })
  const notice = screen.getByRole('status')
  act(() => {
    vi.advanceTimersByTime(3000)
  })
  fireEvent.pointerEnter(notice)
  act(() => {
    vi.advanceTimersByTime(20000)
  })
  expect(screen.getByText('held')).toBeInTheDocument()
  fireEvent.pointerLeave(notice)
  act(() => {
    vi.advanceTimersByTime(4999)
  })
  expect(screen.getByText('held')).toBeInTheDocument()
  act(() => {
    vi.advanceTimersByTime(1)
  })
  expect(screen.queryByText('held')).toBeNull()
})

test('an action closes it without onClose', () => {
  const onClose = vi.fn()
  const undo = vi.fn()
  render(<Notices />)
  act(() => {
    notify({ title: 'gone', actions: [{ label: 'Undo', run: undo }], onClose })
  })
  fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
  expect(undo).toHaveBeenCalled()
  expect(onClose).not.toHaveBeenCalled()
  expect(screen.queryByText('gone')).toBeNull()
})
