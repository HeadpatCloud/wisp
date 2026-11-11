import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { BUILT_IN_THEMES, parseThemeFile, storedTheme, TOKENS } from './themes'

// jsdom has no CSS.supports; this one accepts what the theme files here use.
beforeEach(() => {
  vi.stubGlobal('CSS', {
    supports: (_p: string, v: string) => /^(#[0-9a-f]{3,8}|oklch\(.+\)|rgb\(.+\))$/i.test(v),
  })
})
afterEach(() => vi.unstubAllGlobals())

const graphite = BUILT_IN_THEMES.find((t) => t.id === 'graphite')
if (!graphite) throw new Error('no Graphite')
const file = (patch: Record<string, unknown> = {}) =>
  JSON.stringify({
    wispTheme: 1,
    name: 'Mine',
    kind: 'dark',
    tint: 200,
    colors: graphite.colors,
    terminal: graphite.terminal,
    ...patch,
  })

test('theme_files_are_validated_with_the_key_named', () => {
  const ok = parseThemeFile(file())
  expect(ok).toMatchObject({ name: 'Mine', kind: 'dark', tint: 200 })
  expect(Object.keys(ok.colors).sort()).toEqual([...TOKENS].sort())

  const { border, ...noBorder } = graphite.colors
  expect(border).toBeTruthy()
  expect(() => parseThemeFile(file({ colors: noBorder }))).toThrow('colors.border is missing.')
  expect(() =>
    parseThemeFile(file({ terminal: { ...graphite.terminal, red: 'not red' } })),
  ).toThrow("terminal.red isn't a colour.")
  expect(() => parseThemeFile(file({ kind: 'sepia' }))).toThrow('kind must be light or dark.')
  expect(() => parseThemeFile('{"name": "x"}')).toThrow("That isn't a Wisp theme file.")
  expect(() => parseThemeFile('not json')).toThrow("That isn't a Wisp theme file.")
})

test('a migrated tint theme reads as its built-in with the tint', () => {
  const t = storedTheme({
    id: 'graphite-slate',
    name: 'Graphite · Slate',
    kind: 'dark',
    tint: 255,
    base: 'graphite',
  })
  expect(t).toMatchObject({
    id: 'graphite-slate',
    name: 'Graphite · Slate',
    kind: 'dark',
    tint: 255,
  })
  expect(t?.colors).toEqual(graphite.colors)
  expect(storedTheme({ id: 'x', name: 'broken' })).toBeNull()
})
