import type { FitAddon } from '@xterm/addon-fit'
import type { FontWeight, ITheme, Terminal } from '@xterm/xterm'
import { type AppTheme, resolveDark } from '@/lib/theme'

export const DARK_THEME: ITheme = {
  background: '#1e1e2e',
  foreground: '#cdd6f4',
  cursor: '#f5e0dc',
  cursorAccent: '#1e1e2e',
  selectionBackground: '#585b7066',
  black: '#45475a',
  red: '#f38ba8',
  green: '#a6e3a1',
  yellow: '#f9e2af',
  blue: '#89b4fa',
  magenta: '#f5c2e7',
  cyan: '#94e2d5',
  white: '#bac2de',
  brightBlack: '#585b70',
  brightRed: '#f38ba8',
  brightGreen: '#a6e3a1',
  brightYellow: '#f9e2af',
  brightBlue: '#89b4fa',
  brightMagenta: '#f5c2e7',
  brightCyan: '#94e2d5',
  brightWhite: '#a6adc8',
}

export const LIGHT_THEME: ITheme = {
  background: '#eff1f5',
  foreground: '#4c4f69',
  cursor: '#dc8a78',
  cursorAccent: '#eff1f5',
  selectionBackground: '#acb0be66',
  black: '#5c5f77',
  red: '#d20f39',
  green: '#40a02b',
  yellow: '#df8e1d',
  blue: '#1e66f5',
  magenta: '#ea76cb',
  cyan: '#179299',
  white: '#acb0be',
  brightBlack: '#6c6f85',
  brightRed: '#d20f39',
  brightGreen: '#40a02b',
  brightYellow: '#df8e1d',
  brightBlue: '#1e66f5',
  brightMagenta: '#ea76cb',
  brightCyan: '#179299',
  brightWhite: '#bcc0cc',
}

const DRACULA: ITheme = {
  background: '#282a36',
  foreground: '#f8f8f2',
  cursor: '#f8f8f2',
  selectionBackground: '#44475a',
  black: '#21222c',
  red: '#ff5555',
  green: '#50fa7b',
  yellow: '#f1fa8c',
  blue: '#bd93f9',
  magenta: '#ff79c6',
  cyan: '#8be9fd',
  white: '#f8f8f2',
  brightBlack: '#6272a4',
  brightRed: '#ff6e6e',
  brightGreen: '#69ff94',
  brightYellow: '#ffffa5',
  brightBlue: '#d6acff',
  brightMagenta: '#ff92df',
  brightCyan: '#a4ffff',
  brightWhite: '#ffffff',
}

const NORD: ITheme = {
  background: '#2e3440',
  foreground: '#d8dee9',
  cursor: '#d8dee9',
  selectionBackground: '#434c5e',
  black: '#3b4252',
  red: '#bf616a',
  green: '#a3be8c',
  yellow: '#ebcb8b',
  blue: '#81a1c1',
  magenta: '#b48ead',
  cyan: '#88c0d0',
  white: '#e5e9f0',
  brightBlack: '#4c566a',
  brightRed: '#bf616a',
  brightGreen: '#a3be8c',
  brightYellow: '#ebcb8b',
  brightBlue: '#81a1c1',
  brightMagenta: '#b48ead',
  brightCyan: '#8fbcbb',
  brightWhite: '#eceff4',
}

const GRUVBOX: ITheme = {
  background: '#282828',
  foreground: '#ebdbb2',
  cursor: '#ebdbb2',
  selectionBackground: '#504945',
  black: '#282828',
  red: '#cc241d',
  green: '#98971a',
  yellow: '#d79921',
  blue: '#458588',
  magenta: '#b16286',
  cyan: '#689d6a',
  white: '#a89984',
  brightBlack: '#928374',
  brightRed: '#fb4934',
  brightGreen: '#b8bb26',
  brightYellow: '#fabd2f',
  brightBlue: '#83a598',
  brightMagenta: '#d3869b',
  brightCyan: '#8ec07c',
  brightWhite: '#ebdbb2',
}

// "default" follows the app light/dark theme; named schemes are fixed palettes.
export const COLOR_SCHEMES: { value: string; label: string }[] = [
  { value: 'default', label: 'Default (Catppuccin)' },
  { value: 'dracula', label: 'Dracula' },
  { value: 'nord', label: 'Nord' },
  { value: 'gruvbox', label: 'Gruvbox' },
]

const FIXED_SCHEMES: Record<string, ITheme> = {
  dracula: DRACULA,
  nord: NORD,
  gruvbox: GRUVBOX,
}

export function resolveTerminalTheme(theme: AppTheme, colorScheme: string): ITheme {
  return FIXED_SCHEMES[colorScheme] ?? (resolveDark(theme) ? DARK_THEME : LIGHT_THEME)
}

export interface AppearanceSettings {
  theme: AppTheme
  colorScheme: string
  fontFamily: string
  fontSize: number
  fontWeight: string
  lineHeight: number
  letterSpacing: number
  cursorStyle: 'block' | 'bar' | 'underline'
  cursorBlink: boolean
  scrollback: number
}

// Apply live appearance changes to an existing terminal, refitting only when a
// metric that affects cell size changed.
export function applyTerminalSettings(term: Terminal, fit: FitAddon, s: AppearanceSettings): void {
  const metricsChanged =
    term.options.fontFamily !== s.fontFamily ||
    term.options.fontSize !== s.fontSize ||
    term.options.fontWeight !== s.fontWeight ||
    term.options.lineHeight !== s.lineHeight ||
    term.options.letterSpacing !== s.letterSpacing
  // Assign a NEW theme object - xterm uses reference comparison to detect changes.
  term.options.theme = { ...resolveTerminalTheme(s.theme, s.colorScheme) }
  term.options.fontFamily = s.fontFamily
  term.options.fontSize = s.fontSize
  term.options.fontWeight = s.fontWeight as FontWeight
  term.options.lineHeight = s.lineHeight
  term.options.letterSpacing = s.letterSpacing
  term.options.cursorStyle = s.cursorStyle
  term.options.cursorBlink = s.cursorBlink
  term.options.scrollback = s.scrollback
  if (metricsChanged) fit.fit()
}
