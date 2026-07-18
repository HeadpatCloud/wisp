import { renderHook } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import {
  chordFor,
  chordOf,
  conflictFor,
  formatChord,
  HOTKEY_ACTIONS,
  suspendHotkeys,
  useHotkeys,
} from './hotkeys'

afterEach(() => suspendHotkeys(false))

function press(init: KeyboardEventInit & { key: string }, target?: EventTarget) {
  const ev = new KeyboardEvent('keydown', { ...init, bubbles: true, cancelable: true })
  ;(target ?? window).dispatchEvent(ev)
  return ev
}

test('chordOf normalizes modifiers and case', () => {
  expect(chordOf({ ctrlKey: true, metaKey: false, altKey: false, shiftKey: true, key: 'P' })).toBe(
    'ctrl+shift+p',
  )
  expect(chordOf({ ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, key: '=' })).toBe(
    'ctrl+=',
  )
})

test('chordFor falls back to the default until overridden', () => {
  expect(chordFor('palette', {})).toBe('ctrl+shift+p')
  expect(chordFor('palette', { palette: 'ctrl+k' })).toBe('ctrl+k')
})

test('formatChord renders arrows readably', () => {
  expect(formatChord('ctrl+alt+arrowright')).toBe('Ctrl+Alt+→')
})

test('fires the bound action and swallows the event', () => {
  const palette = vi.fn()
  renderHook(() => useHotkeys({ palette }, {}))
  const ev = press({ key: 'P', ctrlKey: true, shiftKey: true })
  expect(palette).toHaveBeenCalledTimes(1)
  expect(ev.defaultPrevented).toBe(true)
})

test('honours an override instead of the default chord', () => {
  const palette = vi.fn()
  renderHook(() => useHotkeys({ palette }, { palette: 'ctrl+k' }))
  press({ key: 'P', ctrlKey: true, shiftKey: true })
  expect(palette).not.toHaveBeenCalled()
  press({ key: 'k', ctrlKey: true })
  expect(palette).toHaveBeenCalledTimes(1)
})

test('ignores chords typed into inputs but not into the terminal', () => {
  const localShell = vi.fn()
  renderHook(() => useHotkeys({ localShell }, {}))

  const input = document.createElement('input')
  document.body.appendChild(input)
  press({ key: 'T', ctrlKey: true, shiftKey: true }, input)
  expect(localShell).not.toHaveBeenCalled()

  const xterm = document.createElement('textarea')
  xterm.className = 'xterm-helper-textarea'
  document.body.appendChild(xterm)
  press({ key: 'T', ctrlKey: true, shiftKey: true }, xterm)
  expect(localShell).toHaveBeenCalledTimes(1)

  input.remove()
  xterm.remove()
})

// Ctrl+W is readline's kill-word; stealing it would break the terminal.
test('defaults leave bare Ctrl+letter chords to the terminal', () => {
  for (const a of HOTKEY_ACTIONS) {
    const parts = a.default.split('+')
    if (parts.length === 2 && parts[0] === 'ctrl' && /^[a-z]$/.test(parts[1])) {
      throw new Error(`${a.id} claims bare ${a.default}`)
    }
  }
})

test('conflictFor reports the action already holding a chord', () => {
  expect(conflictFor('palette', 'ctrl+shift+w', {})).toBe('closeTab')
  expect(conflictFor('palette', 'ctrl+shift+p', {})).toBeNull()
  expect(conflictFor('closeTab', 'ctrl+shift+w', {})).toBeNull()
})

test('suspend stops the global handler while a rebind is captured', () => {
  const palette = vi.fn()
  renderHook(() => useHotkeys({ palette }, {}))
  suspendHotkeys(true)
  press({ key: 'P', ctrlKey: true, shiftKey: true })
  expect(palette).not.toHaveBeenCalled()
  suspendHotkeys(false)
  press({ key: 'P', ctrlKey: true, shiftKey: true })
  expect(palette).toHaveBeenCalledTimes(1)
})
