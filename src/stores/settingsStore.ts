import { create } from 'zustand'
import { type ConnAppearance, commands, type Settings } from '@/bindings'
import type { AppearanceSettings } from '@/features/sessions/terminalTheme'
import { unwrap } from '@/lib/ipc'
import { terminalColorsFor } from '@/lib/schemes'
import { type AppTheme, cacheTheme } from '@/lib/theme'
import { setTransferLimit } from '@/lib/transferQueue'
import { activeTheme, allThemes, applyAppearance, useThemeStore } from '@/stores/themeStore'

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
  maxConcurrentTransfers: 3,
  copyOnSelect: false,
  rightClickPaste: false,
  hotkeys: {},
  restoreSession: true,
}

function asAppTheme(theme: string): AppTheme {
  return theme === 'light' || theme === 'dark' ? theme : 'system'
}

// `connection` and `tag` are terminal-colours ids that win over the Terminal setting.
function colorsFor(settings: Settings, connection?: string | null, tag?: string | null) {
  const catalog = {
    schemes: useThemeStore.getState().schemes,
    themes: allThemes(),
    active: activeTheme(settings),
  }
  return terminalColorsFor({ connection, tag, setting: settings.colorScheme }, catalog)
}

export function appearanceOf(settings: Settings): AppearanceSettings {
  return {
    colors: colorsFor(settings),
    fontFamily: settings.fontFamily,
    fontSize: settings.fontSize,
    // xterm takes numeric weights; it has no "medium".
    fontWeight: settings.fontWeight === 'medium' ? '500' : (settings.fontWeight ?? 'normal'),
    lineHeight: settings.lineHeight ?? 1.0,
    letterSpacing: settings.letterSpacing ?? 0,
    cursorStyle: (settings.cursorStyle ?? 'block') as 'block' | 'bar' | 'underline',
    cursorBlink: settings.cursorBlink ?? true,
    scrollback: settings.scrollback ?? 10000,
  }
}

// Global appearance with an optional per-connection override layered on top.
export function appearanceFor(
  settings: Settings,
  override?: ConnAppearance | null,
  tagColors?: string | null,
): AppearanceSettings {
  const base = {
    ...appearanceOf(settings),
    colors: colorsFor(settings, override?.colors, tagColors),
  }
  if (!override) return base
  return {
    ...base,
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
    applyAppearance(settings)
    cacheTheme(asAppTheme(settings.theme))
    setTransferLimit(settings.maxConcurrentTransfers ?? 3)
    set({ settings, loaded: true })
  },
  update: async (patch) => {
    const next = { ...get().settings, ...patch }
    // Apply immediately so the UI (terminals, preview, chrome) reflects the change,
    // then persist - a save error must not swallow the visible update.
    applyAppearance(next)
    cacheTheme(asAppTheme(next.theme))
    setTransferLimit(next.maxConcurrentTransfers ?? 3)
    set({ settings: next })
    await commands.setSettings(next).then(unwrap).catch(console.error)
  },
}))
