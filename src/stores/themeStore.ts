import { create } from 'zustand'
import { commands, type Settings } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import type { Scheme } from '@/lib/schemes'
import { ACCENTS, resolveDark } from '@/lib/theme'
import { applyTheme, BUILT_IN_THEMES, storedTheme, type Theme } from '@/lib/themes'

interface ThemeState {
  userThemes: Theme[]
  schemes: Scheme[]
  loaded: boolean
  load: () => Promise<void>
  addTheme: (theme: Theme) => Promise<void>
  removeTheme: (id: string) => Promise<void>
  addSchemes: (schemes: Scheme[]) => Promise<void>
  removeScheme: (id: string) => Promise<void>
}

export const useThemeStore = create<ThemeState>()((set, get) => {
  const save = async (userThemes: Theme[], schemes: Scheme[]) => {
    set({ userThemes, schemes })
    unwrap(await commands.setThemes({ version: 1, themes: userThemes, terminalSchemes: schemes }))
  }
  return {
    userThemes: [],
    schemes: [],
    loaded: false,
    load: async () => {
      const file = unwrap(await commands.getThemes())
      set({
        userThemes: (file.themes ?? []).map(storedTheme).filter((t): t is Theme => !!t),
        schemes: (file.terminalSchemes ?? []).filter(
          (s): s is Scheme => !!s && typeof s === 'object' && 'colors' in s && 'id' in s,
        ),
        loaded: true,
      })
    },
    addTheme: (theme) => save([...get().userThemes, theme], get().schemes),
    removeTheme: (id) =>
      save(
        get().userThemes.filter((t) => t.id !== id),
        get().schemes,
      ),
    addSchemes: (schemes) => save(get().userThemes, [...get().schemes, ...schemes]),
    removeScheme: (id) =>
      save(
        get().userThemes,
        get().schemes.filter((s) => s.id !== id),
      ),
  }
})

export function allThemes(): Theme[] {
  return [...BUILT_IN_THEMES, ...useThemeStore.getState().userThemes]
}

// The theme the light/dark setting picks right now; a removed theme falls back to its built-in.
export function activeTheme(settings: Settings): Theme {
  const mode = settings.theme === 'light' || settings.theme === 'dark' ? settings.theme : 'system'
  const dark = resolveDark(mode)
  const id = dark ? (settings.darkTheme ?? 'graphite') : (settings.lightTheme ?? 'paper')
  return allThemes().find((t) => t.id === id) ?? BUILT_IN_THEMES[dark ? 1 : 0]
}

const FONTS: Record<string, string> = {
  inter: "'Inter Variable', ui-sans-serif, system-ui, sans-serif",
  system: 'ui-sans-serif, system-ui, sans-serif',
}

export function applyAppearance(settings: Settings): void {
  const hue = ACCENTS.find((a) => a.value === settings.accent)?.hue ?? 180
  applyTheme(activeTheme(settings), hue)
  document.documentElement.dataset.density = settings.density ?? 'comfortable'
  const font = settings.uiFont ?? 'inter'
  document.documentElement.style.setProperty('--font-sans', FONTS[font] ?? font)
}
