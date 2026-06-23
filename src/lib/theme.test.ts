import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import {
  applyAccent,
  applyAppTheme,
  applyBackground,
  cachedTheme,
  cacheTheme,
  resolveDark,
  watchSystemTheme,
} from './theme'

function mockMatchMedia(matches: boolean) {
  const listeners = new Set<() => void>()
  vi.stubGlobal('matchMedia', (_q: string) => ({
    matches,
    addEventListener: (_t: string, cb: () => void) => listeners.add(cb),
    removeEventListener: (_t: string, cb: () => void) => listeners.delete(cb),
    dispatchEvent: () => {
      listeners.forEach((cb) => {
        cb()
      })
      return true
    },
  }))
  return listeners
}

beforeEach(() => {
  document.documentElement.classList.remove('dark')
  localStorage.clear()
})
afterEach(() => vi.unstubAllGlobals())

test('applyAppTheme toggles dark class for explicit dark/light', () => {
  mockMatchMedia(false)
  applyAppTheme('dark')
  expect(document.documentElement.classList.contains('dark')).toBe(true)
  applyAppTheme('light')
  expect(document.documentElement.classList.contains('dark')).toBe(false)
})

test('system theme follows the OS preference', () => {
  mockMatchMedia(true)
  expect(resolveDark('system')).toBe(true)
  applyAppTheme('system')
  expect(document.documentElement.classList.contains('dark')).toBe(true)
})

test('applyBackground sets --ui-hue from the preset, falling back to the default', () => {
  applyBackground('rose')
  expect(document.documentElement.style.getPropertyValue('--ui-hue')).toBe('15')
  applyBackground('teal')
  expect(document.documentElement.style.getPropertyValue('--ui-hue')).toBe('195')
  applyBackground('nope')
  expect(document.documentElement.style.getPropertyValue('--ui-hue')).toBe('195')
})

test('applyAccent overrides --primary and --ring with the accent hue', () => {
  applyAccent('red')
  expect(document.documentElement.style.getPropertyValue('--primary')).toContain('25')
  expect(document.documentElement.style.getPropertyValue('--ring')).toContain('25')
})

test('cacheTheme round-trips, defaults to system', () => {
  expect(cachedTheme()).toBe('system')
  cacheTheme('dark')
  expect(cachedTheme()).toBe('dark')
})

test('watchSystemTheme re-applies only while theme is system', () => {
  const listeners = mockMatchMedia(true)
  let current: 'light' | 'dark' | 'system' = 'light'
  const stop = watchSystemTheme(() => current)
  // explicit light: a system change must NOT force dark
  listeners.forEach((cb) => {
    cb()
  })
  expect(document.documentElement.classList.contains('dark')).toBe(false)
  // switch to system: now a system change applies
  current = 'system'
  listeners.forEach((cb) => {
    cb()
  })
  expect(document.documentElement.classList.contains('dark')).toBe(true)
  stop()
})
