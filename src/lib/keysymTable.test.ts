import { expect, test } from 'vitest'
import { LEGACY_KEYSYMS } from './keysymTable'

test('holds every entry generated from keysymdef.h', () => {
  expect(LEGACY_KEYSYMS.size).toBe(722)
})

test('maps to legacy keysyms only', () => {
  for (const keysym of LEGACY_KEYSYMS.values()) expect(keysym).toBeLessThan(0x01000000)
})

test('leaves out code points up to 0xFF', () => {
  for (const cp of LEGACY_KEYSYMS.keys()) expect(cp).toBeGreaterThan(0xff)
})
