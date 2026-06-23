import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({ getSettings: vi.fn(), setSettings: vi.fn() }))
vi.mock('@/bindings', () => ({
  commands: { getSettings: m.getSettings, setSettings: m.setSettings },
}))
vi.mock('@/lib/theme', () => ({
  applyAppTheme: vi.fn(),
  applyAccent: vi.fn(),
  applyBackground: vi.fn(),
  cacheTheme: vi.fn(),
}))

import { appearanceOf, useSettingsStore } from './settingsStore'

const settings = {
  theme: 'dark',
  fontFamily: 'JetBrains Mono',
  fontSize: 16,
  colorScheme: 'default',
}

beforeEach(() => {
  vi.clearAllMocks()
  useSettingsStore.setState({
    settings: { theme: 'system', fontFamily: 'monospace', fontSize: 14, colorScheme: 'default' },
    loaded: false,
  })
})

test('load pulls settings and marks loaded', async () => {
  m.getSettings.mockResolvedValue({ status: 'ok', data: settings })
  await useSettingsStore.getState().load()
  expect(useSettingsStore.getState().settings.fontSize).toBe(16)
  expect(useSettingsStore.getState().loaded).toBe(true)
})

test('update merges a patch and persists', async () => {
  m.setSettings.mockResolvedValue({ status: 'ok', data: null })
  await useSettingsStore.getState().update({ fontSize: 20 })
  expect(m.setSettings).toHaveBeenCalledWith(
    expect.objectContaining({ fontSize: 20, theme: 'system' }),
  )
  expect(useSettingsStore.getState().settings.fontSize).toBe(20)
})

test('appearanceOf coerces an unknown theme to system', () => {
  expect(appearanceOf({ ...settings, theme: 'weird' } as never).theme).toBe('system')
  expect(appearanceOf(settings as never).theme).toBe('dark')
})
