import { afterEach, expect, test, vi } from 'vitest'
import type { AppearanceSettings } from './terminalTheme'
import { applyTerminalSettings, LIGHT_THEME } from './terminalTheme'

const base: AppearanceSettings = {
  colors: LIGHT_THEME,
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
  const element = { getClientRects: () => [{}] }
  const term = { options, element } as never as import('@xterm/xterm').Terminal

  applyTerminalSettings(term, fit as never, base) // theme-only: no metric change
  expect(options.theme).toEqual(LIGHT_THEME)
  expect(options.theme).not.toBe(LIGHT_THEME) // a NEW object, not the constant
  expect(fit.fit).not.toHaveBeenCalled()

  applyTerminalSettings(term, fit as never, { ...base, fontFamily: 'JetBrains Mono', fontSize: 16 })
  expect(options.fontFamily).toBe('JetBrains Mono')
  expect(options.fontSize).toBe(16)
  expect(fit.fit).toHaveBeenCalledTimes(1)
})

test('applyTerminalSettings leaves a hidden terminal unfitted', () => {
  vi.stubGlobal('matchMedia', () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  }))
  const fit = { fit: vi.fn() }
  const options = { fontFamily: 'monospace', fontSize: 14 }
  const element = { getClientRects: () => [] }
  const term = { options, element } as never as import('@xterm/xterm').Terminal
  applyTerminalSettings(term, fit as never, { ...base, fontSize: 18 })
  expect(options.fontSize).toBe(18)
  expect(fit.fit).not.toHaveBeenCalled()
})
