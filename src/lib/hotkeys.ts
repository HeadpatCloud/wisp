import { useEffect, useRef } from 'react'
import { isMac } from '@/lib/platform'

export const HOTKEY_ACTIONS = [
  { id: 'palette', label: 'Command palette', default: 'ctrl+shift+p' },
  // While a terminal has focus a Ctrl chord goes to the shell; Cmd+K on macOS never reaches it.
  {
    id: 'paletteShort',
    label: 'Command palette (short)',
    default: isMac ? 'meta+k' : 'ctrl+k',
    shellFirst: true,
  },
  { id: 'newConnection', label: 'New connection', default: 'ctrl+shift+n' },
  { id: 'toggleHostList', label: 'Show or hide the host list', default: 'ctrl+shift+b' },
  { id: 'localShell', label: 'New local shell', default: 'ctrl+shift+t' },
  // Chords stay shift/alt-qualified: bare Ctrl+letter belongs to readline in the terminal.
  { id: 'closeTab', label: 'Close tab', default: 'ctrl+shift+w' },
  { id: 'nextTab', label: 'Next tab', default: 'ctrl+tab' },
  { id: 'prevTab', label: 'Previous tab', default: 'ctrl+shift+tab' },
  { id: 'moveTabLeft', label: 'Move tab left', default: 'ctrl+shift+pageup' },
  { id: 'moveTabRight', label: 'Move tab right', default: 'ctrl+shift+pagedown' },
  { id: 'splitRight', label: 'Split pane right', default: 'ctrl+alt+arrowright' },
  { id: 'splitDown', label: 'Split pane down', default: 'ctrl+alt+arrowdown' },
  { id: 'closePane', label: 'Close pane', default: 'ctrl+shift+x' },
  { id: 'zoomIn', label: 'Zoom in', default: 'ctrl+plus' },
  { id: 'zoomOut', label: 'Zoom out', default: 'ctrl+-' },
  { id: 'zoomReset', label: 'Reset zoom', default: 'ctrl+0' },
  { id: 'broadcast', label: 'Type into every pane of the tab', default: 'ctrl+shift+i' },
  { id: 'settings', label: 'Open settings', default: 'ctrl+,' },
] as const

export type HotkeyAction = (typeof HOTKEY_ACTIONS)[number]['id']

export function chordOf(
  ev: Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'key'>,
): string {
  // A bare "+" would split the chord, and the plus key needs Shift on some layouts and not on
  // others, so "+" and "=" are one Shift-less token.
  const plus = ev.key === '+' || ev.key === '='
  const parts: string[] = []
  if (ev.ctrlKey) parts.push('ctrl')
  if (ev.metaKey) parts.push('meta')
  if (ev.altKey) parts.push('alt')
  if (ev.shiftKey && !plus) parts.push('shift')
  parts.push(plus ? 'plus' : ev.key.toLowerCase())
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
    .replace('Pageup', 'PgUp')
    .replace('Pagedown', 'PgDn')
}

export function chordFor(action: HotkeyAction, overrides: Record<string, string>): string {
  const chord = overrides[action] ?? HOTKEY_ACTIONS.find((a) => a.id === action)?.default ?? ''
  // Overrides saved before chordOf made "+" and "=" one Shift-less token would never match.
  const key = chord === '+' || chord.endsWith('++') ? '+' : chord.slice(chord.lastIndexOf('+') + 1)
  if (key !== '+' && key !== '=') return chord
  const modifiers = chord
    .slice(0, chord.length - key.length)
    .split('+')
    .filter((m) => m !== '' && m !== 'shift')
  return [...modifiers, 'plus'].join('+')
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

// chord -> action, so a keypress is a single lookup. A chord the user picked beats a default
// that a newer version added on the same keys.
function keymapOf(overrides: Record<string, string>): Record<string, HotkeyAction> {
  const map: Record<string, HotkeyAction> = {}
  for (const a of HOTKEY_ACTIONS) if (!(a.id in overrides)) map[chordFor(a.id, overrides)] = a.id
  for (const a of HOTKEY_ACTIONS) if (a.id in overrides) map[chordFor(a.id, overrides)] = a.id
  return map
}

// Set while the settings UI is capturing a replacement chord, so the global handler
// doesn't swallow the very keypress being recorded.
let suspended = false
export function suspendHotkeys(value: boolean) {
  suspended = value
}

const isTerminal = (t: EventTarget | null) =>
  !!(t as HTMLElement | null)?.classList?.contains('xterm-helper-textarea')

const SHELL_FIRST = new Set<string>(
  HOTKEY_ACTIONS.filter((a) => 'shellFirst' in a).map((a) => a.id),
)

// xterm's hidden textarea must still receive hotkeys; every other text field must not.
function isTypingTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  if (!el) return false
  if (isTerminal(el)) return false
  // A remote desktop gets every key pressed inside it.
  if (el.closest?.('[data-remote-desktop]')) return true
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
      if (SHELL_FIRST.has(action) && ev.ctrlKey && isTerminal(ev.target)) return
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
