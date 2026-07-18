import { useEffect, useRef } from 'react'

export const HOTKEY_ACTIONS = [
  { id: 'palette', label: 'Command palette', default: 'ctrl+shift+p' },
  { id: 'localShell', label: 'New local shell', default: 'ctrl+shift+t' },
  // Chords stay shift/alt-qualified: bare Ctrl+letter belongs to readline in the terminal.
  { id: 'closeTab', label: 'Close tab', default: 'ctrl+shift+w' },
  { id: 'nextTab', label: 'Next tab', default: 'ctrl+tab' },
  { id: 'prevTab', label: 'Previous tab', default: 'ctrl+shift+tab' },
  { id: 'splitRight', label: 'Split pane right', default: 'ctrl+alt+arrowright' },
  { id: 'splitDown', label: 'Split pane down', default: 'ctrl+alt+arrowdown' },
  { id: 'closePane', label: 'Close pane', default: 'ctrl+shift+x' },
  { id: 'zoomIn', label: 'Zoom in', default: 'ctrl+=' },
  { id: 'zoomOut', label: 'Zoom out', default: 'ctrl+-' },
  { id: 'zoomReset', label: 'Reset zoom', default: 'ctrl+0' },
  { id: 'broadcast', label: 'Toggle broadcast input', default: 'ctrl+shift+i' },
  { id: 'settings', label: 'Open settings', default: 'ctrl+,' },
] as const

export type HotkeyAction = (typeof HOTKEY_ACTIONS)[number]['id']

export function chordOf(
  ev: Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'key'>,
): string {
  const parts: string[] = []
  if (ev.ctrlKey) parts.push('ctrl')
  if (ev.metaKey) parts.push('meta')
  if (ev.altKey) parts.push('alt')
  if (ev.shiftKey) parts.push('shift')
  parts.push(ev.key.toLowerCase())
  return parts.join('+')
}

export function formatChord(chord: string): string {
  return chord
    .split('+')
    .map((p) => (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)))
    .join('+')
    .replace('Arrowright', '→')
    .replace('Arrowdown', '↓')
    .replace('Arrowleft', '←')
    .replace('Arrowup', '↑')
}

export function chordFor(action: HotkeyAction, overrides: Record<string, string>): string {
  return overrides[action] ?? HOTKEY_ACTIONS.find((a) => a.id === action)?.default ?? ''
}

// The action already bound to `chord`, so a rebind can't silently shadow another one.
export function conflictFor(
  action: HotkeyAction,
  chord: string,
  overrides: Record<string, string>,
): HotkeyAction | null {
  for (const a of HOTKEY_ACTIONS) {
    if (a.id !== action && chordFor(a.id, overrides) === chord) return a.id
  }
  return null
}

// chord -> action, so a keypress is a single lookup.
function keymapOf(overrides: Record<string, string>): Record<string, HotkeyAction> {
  const map: Record<string, HotkeyAction> = {}
  for (const a of HOTKEY_ACTIONS) map[chordFor(a.id, overrides)] = a.id
  return map
}

// Set while the settings UI is capturing a replacement chord, so the global handler
// doesn't swallow the very keypress being recorded.
let suspended = false
export function suspendHotkeys(value: boolean) {
  suspended = value
}

// xterm's hidden textarea must still receive hotkeys; every other text field must not.
function isTypingTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  if (!el) return false
  if (el.classList?.contains('xterm-helper-textarea')) return false
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable
}

export function useHotkeys(
  handlers: Partial<Record<HotkeyAction, () => void>>,
  overrides: Record<string, string>,
) {
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers
  const mapRef = useRef(keymapOf(overrides))
  mapRef.current = keymapOf(overrides)

  useEffect(() => {
    // Capture phase so a chord reaches us before xterm swallows it into the PTY.
    const onKey = (ev: KeyboardEvent) => {
      if (suspended || isTypingTarget(ev.target)) return
      const action = mapRef.current[chordOf(ev)]
      if (!action) return
      const run = handlersRef.current[action]
      if (!run) return
      ev.preventDefault()
      ev.stopPropagation()
      run()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])
}
