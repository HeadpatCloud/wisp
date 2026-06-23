import { afterEach, expect, test, vi } from 'vitest'
import type { AppearanceSettings } from './terminalTheme'
import {
  applyTerminalSettings,
  DARK_THEME,
  LIGHT_THEME,
  resolveTerminalTheme,
} from './terminalTheme'

const base: AppearanceSettings = {
  theme: 'light',
  colorScheme: 'default',
  fontFamily: 'monospace',
  fontSize: 14,
  fontWeight: 'normal',
  lineHeight: 1,
  letterSpacing: 0,
  cursorStyle: 'block',
  cursorBlink: true,
  scrollback: 10000,
}

afterEach(() => vi.unstubAllGlobals())

test('resolveTerminalTheme maps dark/light/system and named schemes', () => {
  vi.stubGlobal('matchMedia', () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  }))
  expect(resolveTerminalTheme('dark', 'default')).toBe(DARK_THEME)
  expect(resolveTerminalTheme('light', 'default')).toBe(LIGHT_THEME)
  expect(resolveTerminalTheme('system', 'default')).toBe(DARK_THEME) // system prefers dark here
  // a fixed named scheme ignores the app theme
  expect(resolveTerminalTheme('light', 'dracula')).not.toBe(LIGHT_THEME)
})

test('applyTerminalSettings sets a new theme object and refits only on font change', () => {
  vi.stubGlobal('matchMedia', () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  }))
  const fit = { fit: vi.fn() }
  const options = {
    theme: undefined as unknown,
    fontFamily: 'monospace',
    fontSize: 14,
    fontWeight: 'normal',
    lineHeight: 1,
    letterSpacing: 0,
  }
  const term = { options } as never as import('@xterm/xterm').Terminal

  applyTerminalSettings(term, fit as never, base) // theme-only: no metric change
  expect(options.theme).toEqual(LIGHT_THEME)
  expect(options.theme).not.toBe(LIGHT_THEME) // a NEW object, not the constant
  expect(fit.fit).not.toHaveBeenCalled()

  applyTerminalSettings(term, fit as never, { ...base, fontFamily: 'JetBrains Mono', fontSize: 16 })
  expect(options.fontFamily).toBe('JetBrains Mono')
  expect(options.fontSize).toBe(16)
  expect(fit.fit).toHaveBeenCalledTimes(1)
})
