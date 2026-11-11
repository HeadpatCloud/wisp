import { DARK_THEME, LIGHT_THEME, NORD } from '@/features/sessions/terminalTheme'

// Exactly the colour tokens index.css defines.
export const TOKENS = [
  'background',
  'foreground',
  'card',
  'card-foreground',
  'popover',
  'popover-foreground',
  'primary',
  'primary-foreground',
  'secondary',
  'secondary-foreground',
  'muted',
  'muted-foreground',
  'accent',
  'accent-foreground',
  'destructive',
  'destructive-foreground',
  'border',
  'input',
  'ring',
  'sidebar',
] as const
export type Token = (typeof TOKENS)[number]

export const TERMINAL_KEYS = [
  'background',
  'foreground',
  'cursor',
  'selectionBackground',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const
export type TerminalColors = Record<(typeof TERMINAL_KEYS)[number], string>

export interface Theme {
  id: string
  name: string
  kind: 'light' | 'dark'
  // Hue of the neutral surfaces, which read it as --ui-hue.
  tint: number
  colors: Record<Token, string>
  terminal: TerminalColors
  builtIn?: boolean
}

const terminal = (t: Partial<TerminalColors>): TerminalColors =>
  Object.fromEntries(TERMINAL_KEYS.map((k) => [k, t[k] ?? ''])) as TerminalColors

const h = 'var(--ui-hue)'
const PAPER: Record<Token, string> = {
  background: `oklch(0.99 0.004 ${h})`,
  foreground: `oklch(0.16 0.012 ${h})`,
  card: 'oklch(1 0 0)',
  'card-foreground': `oklch(0.16 0.012 ${h})`,
  popover: 'oklch(1 0 0)',
  'popover-foreground': `oklch(0.16 0.012 ${h})`,
  primary: 'oklch(0.6 0.118 180)',
  'primary-foreground': 'oklch(0.99 0.012 180)',
  secondary: `oklch(0.95 0.018 ${h})`,
  'secondary-foreground': `oklch(0.22 0.022 ${h})`,
  muted: `oklch(0.96 0.008 ${h})`,
  'muted-foreground': `oklch(0.5 0.02 ${h})`,
  accent: `oklch(0.92 0.046 ${h})`,
  'accent-foreground': `oklch(0.22 0.03 ${h})`,
  destructive: 'oklch(0.58 0.21 25)',
  'destructive-foreground': 'oklch(0.99 0 0)',
  border: `oklch(0.9 0.012 ${h})`,
  input: `oklch(0.9 0.012 ${h})`,
  ring: 'oklch(0.6 0.118 180)',
  sidebar: `oklch(0.975 0.015 ${h})`,
}
const GRAPHITE: Record<Token, string> = {
  background: `oklch(0.17 0.012 ${h})`,
  foreground: `oklch(0.96 0.006 ${h})`,
  card: `oklch(0.205 0.014 ${h})`,
  'card-foreground': `oklch(0.96 0.006 ${h})`,
  popover: `oklch(0.225 0.016 ${h})`,
  'popover-foreground': `oklch(0.96 0.006 ${h})`,
  primary: 'oklch(0.72 0.13 180)',
  'primary-foreground': 'oklch(0.18 0.022 200)',
  secondary: `oklch(0.27 0.02 ${h})`,
  'secondary-foreground': `oklch(0.96 0.006 ${h})`,
  muted: `oklch(0.27 0.015 ${h})`,
  'muted-foreground': `oklch(0.72 0.022 ${h})`,
  accent: `oklch(0.32 0.05 ${h})`,
  'accent-foreground': `oklch(0.96 0.01 ${h})`,
  destructive: 'oklch(0.62 0.2 25)',
  'destructive-foreground': 'oklch(0.98 0 0)',
  border: `oklch(0.3 0.016 ${h})`,
  input: `oklch(0.3 0.016 ${h})`,
  ring: 'oklch(0.72 0.13 180)',
  sidebar: `oklch(0.2 0.016 ${h})`,
}
// Graphite with more of the tint in every neutral, so the blue reads.
const MIDNIGHT: Record<Token, string> = {
  ...GRAPHITE,
  background: `oklch(0.18 0.035 ${h})`,
  card: `oklch(0.215 0.04 ${h})`,
  popover: `oklch(0.235 0.042 ${h})`,
  secondary: `oklch(0.28 0.045 ${h})`,
  muted: `oklch(0.28 0.04 ${h})`,
  'muted-foreground': `oklch(0.74 0.04 ${h})`,
  border: `oklch(0.32 0.04 ${h})`,
  input: `oklch(0.32 0.04 ${h})`,
  sidebar: `oklch(0.205 0.04 ${h})`,
}
const CONTRAST: Record<Token, string> = {
  ...PAPER,
  background: 'oklch(1 0 0)',
  foreground: 'oklch(0.1 0 0)',
  'card-foreground': 'oklch(0.1 0 0)',
  'popover-foreground': 'oklch(0.1 0 0)',
  'secondary-foreground': 'oklch(0.1 0 0)',
  'muted-foreground': 'oklch(0.3 0 0)',
  'accent-foreground': 'oklch(0.1 0 0)',
  border: 'oklch(0.35 0 0)',
  input: 'oklch(0.35 0 0)',
  sidebar: 'oklch(0.97 0 0)',
}

export const BUILT_IN_THEMES: Theme[] = [
  {
    id: 'paper',
    name: 'Paper',
    kind: 'light',
    tint: 195,
    colors: PAPER,
    terminal: terminal(LIGHT_THEME),
    builtIn: true,
  },
  {
    id: 'graphite',
    name: 'Graphite',
    kind: 'dark',
    tint: 195,
    colors: GRAPHITE,
    terminal: terminal(DARK_THEME),
    builtIn: true,
  },
  {
    id: 'midnight',
    name: 'Midnight',
    kind: 'dark',
    tint: 255,
    colors: MIDNIGHT,
    terminal: terminal(NORD),
    builtIn: true,
  },
  {
    id: 'contrast',
    name: 'High contrast',
    kind: 'light',
    tint: 195,
    colors: CONTRAST,
    terminal: terminal({
      ...LIGHT_THEME,
      background: '#ffffff',
      foreground: '#000000',
      cursor: '#000000',
    }),
    builtIn: true,
  },
]

const isColour = (v: unknown): v is string => typeof v === 'string' && CSS.supports('color', v)
const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object'

function colours<K extends string>(
  raw: unknown,
  keys: readonly K[],
  where: string,
): Record<K, string> {
  const obj = isObject(raw) ? raw : {}
  const out = {} as Record<K, string>
  for (const k of keys) {
    const v = obj[k]
    if (v === undefined) throw new Error(`${where}.${k} is missing.`)
    if (!isColour(v)) throw new Error(`${where}.${k} isn't a colour.`)
    out[k] = v
  }
  return out
}

// A Wisp theme file; the error names the first key that is missing or not a colour.
export function parseThemeFile(text: string): Theme {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    throw new Error("That isn't a Wisp theme file.")
  }
  if (!isObject(raw) || raw.wispTheme !== 1) throw new Error("That isn't a Wisp theme file.")
  if (typeof raw.name !== 'string' || !raw.name.trim()) throw new Error('name is missing.')
  if (raw.kind !== 'light' && raw.kind !== 'dark') throw new Error('kind must be light or dark.')
  if (typeof raw.tint !== 'number') throw new Error('tint is missing.')
  return {
    id: crypto.randomUUID(),
    name: raw.name.trim(),
    kind: raw.kind,
    tint: raw.tint,
    colors: colours(raw.colors, TOKENS, 'colors'),
    terminal: colours(raw.terminal, TERMINAL_KEYS, 'terminal'),
  }
}

export function themeFile(t: Theme): string {
  const { name, kind, tint, colors, terminal } = t
  return `${JSON.stringify({ wispTheme: 1, name, kind, tint, colors, terminal }, null, 2)}\n`
}

// A theme as themes.json holds it: complete, or (made by the 0.5.0 migration) a built-in
// named by `base` with its own tint. Anything else is left out.
export function storedTheme(v: unknown): Theme | null {
  if (!isObject(v) || typeof v.id !== 'string' || typeof v.name !== 'string') return null
  const base = BUILT_IN_THEMES.find((t) => t.id === v.base)
  if (base) {
    return {
      ...base,
      id: v.id,
      name: v.name,
      tint: typeof v.tint === 'number' ? v.tint : base.tint,
      builtIn: false,
    }
  }
  if ((v.kind !== 'light' && v.kind !== 'dark') || typeof v.tint !== 'number') return null
  if (!isObject(v.colors) || !isObject(v.terminal)) return null
  if (TOKENS.some((k) => typeof (v.colors as Record<string, unknown>)[k] !== 'string')) return null
  return v as unknown as Theme
}

// Every token goes on <html>, so a theme replaces the whole palette. The accent still wins
// for --primary and --ring.
export function applyTheme(t: Theme, accentHue: number): void {
  const root = document.documentElement
  for (const k of TOKENS) root.style.setProperty(`--${k}`, t.colors[k])
  root.style.setProperty('--ui-hue', String(t.tint))
  root.classList.toggle('dark', t.kind === 'dark')
  root.style.setProperty('--primary', `oklch(0.66 0.13 ${accentHue})`)
  root.style.setProperty('--ring', `oklch(0.66 0.13 ${accentHue})`)
}
