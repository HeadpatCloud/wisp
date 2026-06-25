import { create } from 'zustand'
import { commands, type ProfileAppearance, type Settings } from '@/bindings'
import type { AppearanceSettings } from '@/features/sessions/terminalTheme'
import { unwrap } from '@/lib/ipc'
import { type AppTheme, applyAccent, applyAppTheme, applyBackground, cacheTheme } from '@/lib/theme'

const DEFAULTS: Settings = {
  theme: 'system',
  fontFamily: 'monospace',
  fontSize: 14,
  colorScheme: 'default',
  accent: 'teal',
  background: 'teal',
  fontWeight: 'normal',
  lineHeight: 1.0,
  letterSpacing: 0,
  cursorStyle: 'block',
  cursorBlink: true,
  scrollback: 10000,
  vncClipboardSync: false,
}

function asAppTheme(theme: string): AppTheme {
  return theme === 'light' || theme === 'dark' ? theme : 'system'
}

export function appearanceOf(settings: Settings): AppearanceSettings {
  return {
    theme: asAppTheme(settings.theme),
    colorScheme: settings.colorScheme,
    fontFamily: settings.fontFamily,
    fontSize: settings.fontSize,
    fontWeight: settings.fontWeight ?? 'normal',
    lineHeight: settings.lineHeight ?? 1.0,
    letterSpacing: settings.letterSpacing ?? 0,
    cursorStyle: (settings.cursorStyle ?? 'block') as 'block' | 'bar' | 'underline',
    cursorBlink: settings.cursorBlink ?? true,
    scrollback: settings.scrollback ?? 10000,
  }
}

// Global appearance with an optional per-profile override layered on top.
export function appearanceFor(
  settings: Settings,
  override?: ProfileAppearance | null,
): AppearanceSettings {
  const base = appearanceOf(settings)
  if (!override) return base
  return {
    ...base,
    theme: override.theme ? asAppTheme(override.theme) : base.theme,
    fontFamily: override.fontFamily ?? base.fontFamily,
    fontSize: override.fontSize ?? base.fontSize,
  }
}

interface SettingsState {
  settings: Settings
  loaded: boolean
  load: () => Promise<void>
  update: (patch: Partial<Settings>) => Promise<void>
}

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  settings: DEFAULTS,
  loaded: false,
  load: async () => {
    const settings = unwrap(await commands.getSettings())
    applyAppTheme(asAppTheme(settings.theme))
    applyAccent(settings.accent ?? 'teal')
    applyBackground(settings.background ?? 'teal')
    cacheTheme(asAppTheme(settings.theme))
    set({ settings, loaded: true })
  },
  update: async (patch) => {
    const next = { ...get().settings, ...patch }
    // Apply immediately so the UI (terminals, preview, chrome) reflects the change,
    // then persist - a save error must not swallow the visible update.
    applyAppTheme(asAppTheme(next.theme))
    applyAccent(next.accent ?? 'teal')
    applyBackground(next.background ?? 'teal')
    cacheTheme(asAppTheme(next.theme))
    set({ settings: next })
    await commands.setSettings(next).then(unwrap).catch(console.error)
  },
}))
