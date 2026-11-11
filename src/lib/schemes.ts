import { DRACULA, GRUVBOX, NORD } from '@/features/sessions/terminalTheme'
import { TERMINAL_KEYS, type TerminalColors, type Theme } from './themes'

export interface Scheme {
  id: string
  name: string
  colors: TerminalColors
  builtIn?: boolean
}

const filled = (t: Partial<TerminalColors>): TerminalColors =>
  Object.fromEntries(TERMINAL_KEYS.map((k) => [k, t[k] ?? ''])) as TerminalColors

export const BUILT_IN_SCHEMES: Scheme[] = [
  { id: 'dracula', name: 'Dracula', colors: filled(DRACULA), builtIn: true },
  { id: 'nord', name: 'Nord', colors: filled(NORD), builtIn: true },
  { id: 'gruvbox', name: 'Gruvbox', colors: filled(GRUVBOX), builtIn: true },
]

// Windows Terminal says purple for magenta and cursorColor for the cursor.
const WT_NAMES: Record<string, string> = {
  magenta: 'purple',
  brightMagenta: 'brightPurple',
  cursor: 'cursorColor',
}

function wtScheme(raw: Record<string, unknown>): Scheme {
  const fg = raw.foreground
  // Both are optional in Windows Terminal, which then draws them from the foreground.
  const optional: Record<string, unknown> = {
    cursor: fg,
    selectionBackground: typeof fg === 'string' && /^#[0-9a-f]{6}$/i.test(fg) ? `${fg}66` : fg,
  }
  const named = typeof raw.name === 'string' ? `${raw.name}: ` : ''
  const colors = {} as TerminalColors
  for (const k of TERMINAL_KEYS) {
    const v = raw[WT_NAMES[k] ?? k] ?? optional[k]
    if (typeof v !== 'string') throw new Error(`${named}${WT_NAMES[k] ?? k} is missing.`)
    colors[k] = v
  }
  return {
    id: crypto.randomUUID(),
    name: typeof raw.name === 'string' ? raw.name : 'Imported',
    colors,
  }
}

// Windows Terminal's settings.json allows comments and trailing commas, which JSON.parse refuses.
function plainJson(text: string): string {
  let out = ''
  let inString = false
  for (let i = 0; i < text.length; i++) {
    const c = text[i]
    if (inString) {
      out += c
      if (c === '\\') out += text[++i] ?? ''
      else if (c === '"') inString = false
    } else if (c === '"') {
      inString = true
      out += c
    } else if (c === '/' && text[i + 1] === '/') {
      while (i + 1 < text.length && text[i + 1] !== '\n') i++
    } else if (c === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2)
      i = end < 0 ? text.length : end + 1
    } else if (c === '}' || c === ']') {
      out = out.replace(/,\s*$/, '') + c
    } else {
      out += c
    }
  }
  return out
}

// One scheme object, or a settings file with a `schemes` array.
export function parseWindowsTerminal(text: string): Scheme[] {
  const raw = JSON.parse(plainJson(text))
  const list: unknown[] = Array.isArray(raw?.schemes) ? raw.schemes : [raw]
  return list.map((s) => wtScheme((s ?? {}) as Record<string, unknown>))
}

const ITERM_NAMES: Record<string, string> = {
  background: 'Background Color',
  foreground: 'Foreground Color',
  cursor: 'Cursor Color',
  selectionBackground: 'Selection Color',
  ...Object.fromEntries(TERMINAL_KEYS.slice(4).map((k, i) => [k, `Ansi ${i} Color`])),
}

const hex = (n: number) =>
  Math.round(Math.max(0, Math.min(1, n)) * 255)
    .toString(16)
    .padStart(2, '0')

// An .itermcolors plist: each colour a dict of 0-1 components.
export function parseItermColors(text: string, fileName: string): Scheme {
  const doc = new DOMParser().parseFromString(text, 'application/xml')
  const top = doc.querySelector('plist > dict')
  const entries = new Map<string, Element>()
  const children = top ? [...top.children] : []
  for (let i = 0; i < children.length - 1; i++) {
    if (children[i].tagName === 'key') entries.set(children[i].textContent ?? '', children[i + 1])
  }
  const colors = {} as TerminalColors
  // Missing ANSI colours are named in their own order, the others after.
  for (const k of [...TERMINAL_KEYS.slice(4), ...TERMINAL_KEYS.slice(0, 4)]) {
    const dict = entries.get(ITERM_NAMES[k])
    if (!dict) throw new Error(`${ITERM_NAMES[k]} is missing.`)
    const parts = new Map<string, number>()
    const kids = [...dict.children]
    for (let i = 0; i < kids.length - 1; i++) {
      if (kids[i].tagName === 'key')
        parts.set(kids[i].textContent ?? '', Number(kids[i + 1].textContent))
    }
    const c = (name: string) => parts.get(`${name} Component`) ?? 0
    colors[k] = `#${hex(c('Red'))}${hex(c('Green'))}${hex(c('Blue'))}`
  }
  return { id: crypto.randomUUID(), name: fileName.replace(/\.itermcolors$/i, ''), colors }
}

interface Catalog {
  schemes: Scheme[]
  themes: Theme[]
  active: Theme
}

// A terminal-colours id: a scheme, `theme:<id>`, or 0.5.0's light and dark (Paper, Graphite).
function byId(id: string | null | undefined, c: Catalog): TerminalColors | null {
  if (!id) return null
  const themeId = id === 'light' ? 'paper' : id === 'dark' ? 'graphite' : id.replace(/^theme:/, '')
  if (id === 'light' || id === 'dark' || id.startsWith('theme:')) {
    return c.themes.find((t) => t.id === themeId)?.terminal ?? null
  }
  return [...BUILT_IN_SCHEMES, ...c.schemes].find((s) => s.id === id)?.colors ?? null
}

// The connection's colours, then its first tag with colours, then the Terminal setting, where
// "default" is the active theme's own.
export function terminalColorsFor(
  pick: { connection?: string | null; tag?: string | null; setting: string },
  c: Catalog,
): TerminalColors {
  return (
    byId(pick.connection, c) ??
    byId(pick.tag, c) ??
    (pick.setting === 'default' ? null : byId(pick.setting, c)) ??
    c.active.terminal
  )
}
