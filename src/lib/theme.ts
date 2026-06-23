export type AppTheme = 'light' | 'dark' | 'system'

const CACHE_KEY = 'app-theme'

function systemPrefersDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

export function resolveDark(theme: AppTheme): boolean {
  return theme === 'dark' || (theme === 'system' && systemPrefersDark())
}

export function applyAppTheme(theme: AppTheme): void {
  document.documentElement.classList.toggle('dark', resolveDark(theme))
}

export const ACCENTS: { value: string; label: string; hue: number }[] = [
  { value: 'teal', label: 'Teal', hue: 180 },
  { value: 'blue', label: 'Blue', hue: 255 },
  { value: 'violet', label: 'Violet', hue: 292 },
  { value: 'pink', label: 'Pink', hue: 350 },
  { value: 'red', label: 'Red', hue: 25 },
  { value: 'orange', label: 'Orange', hue: 60 },
  { value: 'green', label: 'Green', hue: 145 },
]

// Recolour the UI accent (primary buttons, active tab, focus ring) by overriding
// the --primary/--ring oklch hue inline (wins over the stylesheet for both themes).
export function applyAccent(accent: string): void {
  const hue = ACCENTS.find((a) => a.value === accent)?.hue ?? 180
  const root = document.documentElement.style
  root.setProperty('--primary', `oklch(0.66 0.13 ${hue})`)
  root.setProperty('--ring', `oklch(0.66 0.13 ${hue})`)
}

// Background tints: hue values for the whole neutral UI palette (background, cards,
// sidebar, borders...). The default 195 keeps the original teal look.
export const BACKGROUNDS: { value: string; label: string; hue: number }[] = [
  { value: 'teal', label: 'Teal', hue: 195 },
  { value: 'slate', label: 'Slate', hue: 255 },
  { value: 'indigo', label: 'Indigo', hue: 285 },
  { value: 'plum', label: 'Plum', hue: 330 },
  { value: 'rose', label: 'Rose', hue: 15 },
  { value: 'sand', label: 'Sand', hue: 75 },
  { value: 'forest', label: 'Forest', hue: 150 },
]

// Re-tint every neutral UI surface at once by setting the single --ui-hue that the
// stylesheet's oklch() colors read from.
export function applyBackground(background: string): void {
  const hue = BACKGROUNDS.find((b) => b.value === background)?.hue ?? 195
  document.documentElement.style.setProperty('--ui-hue', String(hue))
}

export function cacheTheme(theme: AppTheme): void {
  localStorage.setItem(CACHE_KEY, theme)
}

export function cachedTheme(): AppTheme {
  const v = localStorage.getItem(CACHE_KEY)
  return v === 'light' || v === 'dark' || v === 'system' ? v : 'system'
}

export function watchSystemTheme(getTheme: () => AppTheme): () => void {
  const mql = window.matchMedia('(prefers-color-scheme: dark)')
  const onChange = () => {
    if (getTheme() === 'system') applyAppTheme('system')
  }
  mql.addEventListener('change', onChange)
  return () => mql.removeEventListener('change', onChange)
}
