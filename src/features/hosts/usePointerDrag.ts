import { useEffect, useRef, useState } from 'react'

export type DropTarget =
  | { kind: 'folder'; id: string | null }
  | { kind: 'before'; hostId: string }
  | { kind: 'after'; hostId: string }
  | { kind: 'conn'; index: number }

// Tauri swallows HTML5 drag and drop on Windows, so dragging follows the pointer and finds the
// target under it by its data-drop-* attribute.
export function targetAt(x: number, y: number): { target: DropTarget; el: HTMLElement } | null {
  const el = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>('[data-drop-folder],[data-drop-host],[data-drop-conn]')
  if (!el) return null
  const { dropFolder, dropHost, dropConn } = el.dataset
  if (dropFolder !== undefined) return { target: { kind: 'folder', id: dropFolder || null }, el }
  if (dropConn !== undefined) return { target: { kind: 'conn', index: Number(dropConn) }, el }
  if (dropHost !== undefined) {
    const rect = el.getBoundingClientRect()
    const upper = y < rect.top + rect.height / 2
    return { target: { kind: upper ? 'before' : 'after', hostId: dropHost }, el }
  }
  return null
}

const THRESHOLD = 4

type Finder<D> = (x: number, y: number) => { target: D; el: HTMLElement } | null

export function usePointerDrag<T, D = DropTarget>(
  onDrop: (item: T, target: D) => void,
  find: Finder<D> = targetAt as unknown as Finder<D>,
) {
  const [dragging, setDragging] = useState<T | null>(null)
  const drop = useRef(onDrop)
  drop.current = onDrop
  const cleanup = useRef<(() => void) | null>(null)

  useEffect(() => () => cleanup.current?.(), [])

  const start = (
    item: T,
    e: { button: number; clientX: number; clientY: number; target?: EventTarget | null },
  ) => {
    // Selecting text in a rename field is not a drag.
    if (e.button !== 0 || (e.target instanceof Element && e.target.closest('input'))) return
    cleanup.current?.()
    const from = { x: e.clientX, y: e.clientY }
    let moving = false
    let over: HTMLElement | null = null
    const mark = (el: HTMLElement | null) => {
      if (over === el) return
      if (over) delete over.dataset.dropOver
      over = el
      if (over) over.dataset.dropOver = ''
    }
    const onMove = (ev: PointerEvent) => {
      if (!moving && Math.hypot(ev.clientX - from.x, ev.clientY - from.y) < THRESHOLD) return
      if (!moving) {
        moving = true
        setDragging(item)
      }
      mark(find(ev.clientX, ev.clientY)?.el ?? null)
    }
    const onUp = (ev: PointerEvent) => {
      const hit = moving ? find(ev.clientX, ev.clientY) : null
      end()
      if (hit) drop.current(item, hit.target)
    }
    const end = () => {
      mark(null)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      cleanup.current = null
      setDragging(null)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    cleanup.current = end
  }

  return { dragging, start }
}
