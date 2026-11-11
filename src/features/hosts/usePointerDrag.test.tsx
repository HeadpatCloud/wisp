import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { type DropTarget, usePointerDrag } from './usePointerDrag'

function Harness({ onDrop }: { onDrop: (item: string, target: DropTarget) => void }) {
  const drag = usePointerDrag<string>(onDrop)
  return (
    <div>
      <button type="button" onPointerDown={(e) => drag.start('nas', e)}>
        nas
      </button>
      <div data-testid="folder" data-drop-folder="work">
        Work
      </div>
      <div data-testid="all" data-drop-folder="">
        All hosts
      </div>
      <div data-testid="host" data-drop-host="pi">
        pi
      </div>
      {drag.dragging && <p>dragging {drag.dragging}</p>}
    </div>
  )
}

const at = (el: Element, top = 0, height = 20) => {
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({ top, height } as DOMRect)
  document.elementFromPoint = vi.fn().mockReturnValue(el)
}

afterEach(() => {
  vi.restoreAllMocks()
})

function drag(to: { x: number; y: number }) {
  fireEvent.pointerDown(screen.getByRole('button', { name: 'nas' }), {
    button: 0,
    clientX: 0,
    clientY: 0,
  })
  fireEvent.pointerMove(window, { clientX: to.x, clientY: to.y })
  fireEvent.pointerUp(window, { clientX: to.x, clientY: to.y })
}

test('dropping on a folder hands over the folder', () => {
  const onDrop = vi.fn()
  render(<Harness onDrop={onDrop} />)
  at(screen.getByTestId('folder'))
  drag({ x: 30, y: 5 })
  expect(onDrop).toHaveBeenCalledWith('nas', { kind: 'folder', id: 'work' })
})

test('the All hosts target is no folder', () => {
  const onDrop = vi.fn()
  render(<Harness onDrop={onDrop} />)
  at(screen.getByTestId('all'))
  drag({ x: 30, y: 5 })
  expect(onDrop).toHaveBeenCalledWith('nas', { kind: 'folder', id: null })
})

test('dropping on the upper or lower half of a host puts it before or after', () => {
  const onDrop = vi.fn()
  render(<Harness onDrop={onDrop} />)
  at(screen.getByTestId('host'), 100, 40)
  drag({ x: 30, y: 110 })
  expect(onDrop).toHaveBeenLastCalledWith('nas', { kind: 'before', hostId: 'pi' })
  drag({ x: 30, y: 130 })
  expect(onDrop).toHaveBeenLastCalledWith('nas', { kind: 'after', hostId: 'pi' })
})

test('a press without movement is a click, not a drag', () => {
  const onDrop = vi.fn()
  render(<Harness onDrop={onDrop} />)
  at(screen.getByTestId('folder'))
  drag({ x: 2, y: 1 })
  expect(onDrop).not.toHaveBeenCalled()
})

test('a drop outside any target does nothing', () => {
  const onDrop = vi.fn()
  render(<Harness onDrop={onDrop} />)
  document.elementFromPoint = vi.fn().mockReturnValue(document.body)
  drag({ x: 40, y: 40 })
  expect(onDrop).not.toHaveBeenCalled()
})
