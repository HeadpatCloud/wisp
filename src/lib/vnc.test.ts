import { expect, test } from 'vitest'
import { keysymFor, vncButtonMask } from './vnc'

test('vncButtonMask remaps JS button bits to VNC order', () => {
  expect(vncButtonMask(0)).toBe(0)
  expect(vncButtonMask(1)).toBe(1) // left -> bit0
  expect(vncButtonMask(2)).toBe(4) // right -> bit2
  expect(vncButtonMask(4)).toBe(2) // middle -> bit1
  expect(vncButtonMask(1 | 2)).toBe(1 | 4) // left + right
})

test('keysymFor maps printable chars and named keys', () => {
  expect(keysymFor('a')).toBe(0x61)
  expect(keysymFor('Enter')).toBe(0xff0d)
  expect(keysymFor('ArrowLeft')).toBe(0xff51)
  expect(keysymFor('F5')).toBeNull()
})
