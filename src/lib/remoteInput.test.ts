import { describe, expect, it } from 'vitest'
import { HeldKeys, keysymFor, wheelButtons } from './remoteInput'

describe('keysymFor', () => {
  it.each([
    ['ShiftLeft', 'Shift', 0xffe1],
    ['ShiftRight', 'Shift', 0xffe2],
    ['ControlLeft', 'Control', 0xffe3],
    ['ControlRight', 'Control', 0xffe4],
    ['CapsLock', 'CapsLock', 0xffe5],
    ['MetaLeft', 'Meta', 0xffeb],
    ['MetaRight', 'Meta', 0xffec],
    ['AltLeft', 'Alt', 0xffe9],
    ['AltRight', 'Alt', 0xffea],
    ['AltRight', 'AltGraph', 0xfe03],
    ['Enter', 'Enter', 0xff0d],
    ['NumpadEnter', 'Enter', 0xff8d],
    ['Backspace', 'Backspace', 0xff08],
    ['Tab', 'Tab', 0xff09],
    ['Escape', 'Escape', 0xff1b],
    ['Delete', 'Delete', 0xffff],
    ['Insert', 'Insert', 0xff63],
    ['Home', 'Home', 0xff50],
    ['End', 'End', 0xff57],
    ['PageUp', 'PageUp', 0xff55],
    ['PageDown', 'PageDown', 0xff56],
    ['ArrowLeft', 'ArrowLeft', 0xff51],
    ['ArrowUp', 'ArrowUp', 0xff52],
    ['ArrowRight', 'ArrowRight', 0xff53],
    ['ArrowDown', 'ArrowDown', 0xff54],
    ['F1', 'F1', 0xffbe],
    ['F2', 'F2', 0xffbf],
    ['F3', 'F3', 0xffc0],
    ['F4', 'F4', 0xffc1],
    ['F5', 'F5', 0xffc2],
    ['F6', 'F6', 0xffc3],
    ['F7', 'F7', 0xffc4],
    ['F8', 'F8', 0xffc5],
    ['F9', 'F9', 0xffc6],
    ['F10', 'F10', 0xffc7],
    ['F11', 'F11', 0xffc8],
    ['F12', 'F12', 0xffc9],
    ['F13', 'F13', 0xffca],
    ['F14', 'F14', 0xffcb],
    ['F15', 'F15', 0xffcc],
    ['F16', 'F16', 0xffcd],
    ['F17', 'F17', 0xffce],
    ['F18', 'F18', 0xffcf],
    ['F19', 'F19', 0xffd0],
    ['F20', 'F20', 0xffd1],
    ['F21', 'F21', 0xffd2],
    ['F22', 'F22', 0xffd3],
    ['F23', 'F23', 0xffd4],
    ['F24', 'F24', 0xffd5],
    ['PrintScreen', 'PrintScreen', 0xff61],
    ['ScrollLock', 'ScrollLock', 0xff14],
    ['Pause', 'Pause', 0xff13],
    ['NumLock', 'NumLock', 0xff7f],
    ['ContextMenu', 'ContextMenu', 0xff67],
    ['Space', ' ', 0x20],
  ])('maps %s (%s) by code', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['Numpad0', 'Insert', 0xff9e],
    ['Numpad1', 'End', 0xff9c],
    ['Numpad2', 'ArrowDown', 0xff99],
    ['Numpad3', 'PageDown', 0xff9b],
    ['Numpad4', 'ArrowLeft', 0xff96],
    ['Numpad5', 'Clear', 0xff9d],
    ['Numpad6', 'ArrowRight', 0xff98],
    ['Numpad7', 'Home', 0xff95],
    ['Numpad8', 'ArrowUp', 0xff97],
    ['Numpad9', 'PageUp', 0xff9a],
    ['NumpadDecimal', 'Delete', 0xff9f],
  ])('maps %s (%s) with NumLock off', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['Numpad0', '0', 0xffb0],
    ['Numpad1', '1', 0xffb1],
    ['Numpad2', '2', 0xffb2],
    ['Numpad3', '3', 0xffb3],
    ['Numpad4', '4', 0xffb4],
    ['Numpad5', '5', 0xffb5],
    ['Numpad6', '6', 0xffb6],
    ['Numpad7', '7', 0xffb7],
    ['Numpad8', '8', 0xffb8],
    ['Numpad9', '9', 0xffb9],
    ['NumpadDecimal', '.', 0xffae],
    ['NumpadDecimal', ',', 0xffae],
    ['NumpadAdd', '+', 0xffab],
    ['NumpadSubtract', '-', 0xffad],
    ['NumpadMultiply', '*', 0xffaa],
    ['NumpadDivide', '/', 0xffaf],
  ])('maps %s (%s) to its keypad keysym', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['KeyA', 'a', 0x61],
    ['KeyA', 'A', 0x41],
    ['KeyE', '€', 0x010020ac],
    ['KeyQ', 'ä', 0xe4],
    ['KeyE', '😀', 0x0101f600],
    ['Backquote', '~', 0x7e],
    ['Digit1', ' ', 0xa0],
    ['KeyY', 'ÿ', 0xff],
    ['KeyA', 'Ā', 0x01000100],
    ['', ' ', 0x20],
    ['', '\u001f', 0x0100001f],
    ['', '\u007f', 0x0100007f],
    ['', '\u009f', 0x0100009f],
  ])('maps %s (%s) by key', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['BracketLeft', 'Dead'],
    ['', 'Unidentified'],
    ['KeyA', 'Process'],
    ['AudioVolumeUp', 'AudioVolumeUp'],
    ['KeyE', 'é'],
    ['KeyA', ''],
  ])('returns null for %s (%s)', (code, key) => {
    expect(keysymFor({ code, key })).toBeNull()
  })
})

describe('HeldKeys', () => {
  it('releases the keysym that was pressed after Shift was let go', () => {
    const held = new HeldKeys()
    held.press('ShiftLeft', 0xffe1)
    held.press('KeyA', 0x41)
    expect(held.release('ShiftLeft')).toBe(0xffe1)
    expect(held.release('KeyA')).toBe(0x41)
  })

  it('keeps the first keysym on auto-repeat', () => {
    const held = new HeldKeys()
    held.press('KeyA', 0x41)
    held.press('KeyA', 0x61)
    expect(held.release('KeyA')).toBe(0x41)
  })

  it('returns null for a code that is not held', () => {
    const held = new HeldKeys()
    expect(held.release('KeyA')).toBeNull()
    held.press('KeyA', 0x61)
    held.release('KeyA')
    expect(held.release('KeyA')).toBeNull()
  })

  it('releaseAll returns each held keysym once and leaves nothing held', () => {
    const held = new HeldKeys()
    held.press('ControlLeft', 0xffe3)
    held.press('KeyA', 0x61)
    held.press('KeyA', 0x61)
    held.press('KeyB', 0x62)
    held.press('KeyB', 0x62)
    held.release('KeyB')
    expect(held.releaseAll()).toEqual([0xffe3, 0x61])
    expect(held.releaseAll()).toEqual([])
    expect(held.release('ControlLeft')).toBeNull()
    expect(held.release('KeyA')).toBeNull()
  })
})

describe('wheelButtons', () => {
  it.each([
    [0, 0, 0],
    [0, -1, 8],
    [0, 1, 16],
    [-1, 0, 32],
    [1, 0, 64],
    [-1, -1, 40],
    [1, -1, 72],
    [-1, 1, 48],
    [1, 1, 80],
    [0, -120, 8],
    [0, 0.5, 16],
    [-33.3, 0, 32],
    [100, 0, 64],
  ])('maps deltaX %d, deltaY %d to %d', (deltaX, deltaY, mask) => {
    expect(wheelButtons(deltaX, deltaY)).toBe(mask)
  })
})
