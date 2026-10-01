import { expect, test } from 'vitest'
import { LEGACY_KEYSYMS } from './keysymTable'

test('holds every entry generated from keysymdef.h', () => {
  expect(LEGACY_KEYSYMS.size).toBe(729)
})

test('maps to legacy keysyms only', () => {
  for (const keysym of LEGACY_KEYSYMS.values()) expect(keysym).toBeLessThan(0x01000000)
})

test('leaves out code points that are their own keysym', () => {
  for (const cp of LEGACY_KEYSYMS.keys()) {
    expect((cp >= 0x20 && cp <= 0x7e) || (cp >= 0xa0 && cp <= 0xff)).toBe(false)
  }
})
