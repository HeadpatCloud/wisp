import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  detectPlatform,
  HeldKeys,
  keysymFor,
  type Platform,
  RemoteKeyboard,
  WheelSteps,
  wheelButtons,
} from './remoteInput'

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
    ['NumpadDecimal', ',', 0xffac],
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
    ['KeyQ', 'ä', 0xe4],
    ['Backquote', '~', 0x7e],
    ['Digit1', '\u00a0', 0xa0],
    ['KeyY', 'ÿ', 0xff],
    ['', ' ', 0x20],
    ['Space', ' ', 0x20],
    ['Space', '\u00a0', 0xa0],
  ])('maps %s (%j) to its own code point', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['KeyE', '€', 0x20ac],
    ['KeyF', '\u0430', 0x6c1],
    ['KeyA', 'ą', 0x1b1],
    ['KeyA', 'α', 0x7e1],
    ['Minus', '—', 0xaa9],
    ['Digit2', '“', 0xad2],
    ['KeyA', 'Ā', 0x3c0],
  ])('maps %s (%j) to its legacy keysym', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['KeyA', '中', 0x01004e2d],
    ['KeyE', '😀', 0x0101f600],
    ['Space', '\u202f', 0x0100202f],
    ['', '\ud7ff', 0x0100d7ff],
    ['', '\ue000', 0x0100e000],
  ])('maps %s (%j) to its Unicode keysym', (code, key, keysym) => {
    expect(keysymFor({ code, key })).toBe(keysym)
  })

  it.each([
    ['BracketLeft', 'Dead'],
    ['', 'Unidentified'],
    ['KeyA', 'Process'],
    ['AudioVolumeUp', 'AudioVolumeUp'],
    ['Space', 'Unidentified'],
    ['KeyE', 'e\u0301'],
    ['KeyA', ''],
    ['Numpad1', 'Dead'],
    ['Numpad5', 'Process'],
    ['NumpadDecimal', 'Unidentified'],
    ['', '\u0008'],
    ['', '\u001f'],
    ['', '\u007f'],
    ['', '\u009f'],
    ['', '\ud800'],
    ['', '\udfff'],
  ])('returns null for %s (%j)', (code, key) => {
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
    expect(held.press('KeyA', 0x41)).toBe(0x41)
    expect(held.press('KeyA', 0x61)).toBe(0x41)
    expect(held.release('KeyA')).toBe(0x41)
    expect(held.press('KeyA', 0x61)).toBe(0x61)
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

describe('detectPlatform', () => {
  it.each([
    [
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0',
      'windows',
    ],
    [
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)',
      'mac',
    ],
    [
      'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15',
      'other',
    ],
  ])('reads %s as %s', (userAgent, platform) => {
    expect(detectPlatform(userAgent)).toBe(platform)
  })
})

describe('RemoteKeyboard', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function keyboard(platform: Platform) {
    const sent: [boolean, number][] = []
    const kb = new RemoteKeyboard((down, keysym) => {
      sent.push([down, keysym])
    }, platform)
    return { kb, sent }
  }

  it('resends the first keysym on auto-repeat', () => {
    const { kb, sent } = keyboard('other')
    expect(kb.keydown({ code: 'KeyE', key: 'e' })).toBe(true)
    expect(kb.keydown({ code: 'KeyE', key: '€' })).toBe(true)
    expect(kb.keyup({ code: 'KeyE', key: '€' })).toBe(true)
    expect(sent).toEqual([
      [true, 0x65],
      [true, 0x65],
      [false, 0x65],
    ])
    kb.releaseAll()
    expect(sent).toHaveLength(3)
  })

  it('leaves a key without a keysym to the caller', () => {
    const { kb, sent } = keyboard('other')
    expect(kb.keydown({ code: 'BracketLeft', key: 'Dead' })).toBe(false)
    expect(kb.keyup({ code: 'BracketLeft', key: 'Dead' })).toBe(false)
    expect(sent).toEqual([])
  })

  it('consumes the keyup of a key that is not held without sending', () => {
    const { kb, sent } = keyboard('other')
    expect(kb.keyup({ code: 'KeyA', key: 'a' })).toBe(true)
    expect(sent).toEqual([])
  })

  it('releases a held key whose keyup no longer maps', () => {
    const { kb, sent } = keyboard('other')
    kb.keydown({ code: 'KeyE', key: 'e' })
    expect(kb.keyup({ code: 'KeyE', key: 'Dead' })).toBe(true)
    expect(sent).toEqual([
      [true, 0x65],
      [false, 0x65],
    ])
  })

  it.each(['', 'Unidentified'])('clicks keys with code %j at once and never holds them', (code) => {
    const { kb, sent } = keyboard('other')
    expect(kb.keydown({ code, key: 'a' })).toBe(true)
    expect(kb.keydown({ code, key: 'b' })).toBe(true)
    expect(sent).toEqual([
      [true, 0x61],
      [false, 0x61],
      [true, 0x62],
      [false, 0x62],
    ])
    expect(kb.keyup({ code, key: 'a' })).toBe(true)
    expect(kb.keyup({ code, key: 'b' })).toBe(true)
    kb.releaseAll()
    expect(sent).toHaveLength(4)
  })

  it('sends AltGr on Windows without the Control that announces it', () => {
    const { kb, sent } = keyboard('windows')
    expect(kb.keydown({ code: 'ControlLeft', key: 'Control' })).toBe(true)
    expect(kb.keydown({ code: 'AltRight', key: 'AltGraph' })).toBe(true)
    kb.keydown({ code: 'KeyQ', key: '@' })
    kb.keyup({ code: 'KeyQ', key: '@' })
    expect(kb.keyup({ code: 'AltRight', key: 'AltGraph' })).toBe(true)
    expect(kb.keyup({ code: 'ControlLeft', key: 'Control' })).toBe(true)
    vi.advanceTimersByTime(100)
    expect(sent).toEqual([
      [true, 0xfe03],
      [true, 0x40],
      [false, 0x40],
      [false, 0xfe03],
    ])
  })

  it('keeps Control out of a repeating AltGr on Windows', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    kb.keydown({ code: 'AltRight', key: 'AltGraph' })
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    kb.keydown({ code: 'AltRight', key: 'AltGraph' })
    kb.keyup({ code: 'ControlLeft', key: 'Control' })
    kb.keyup({ code: 'AltRight', key: 'AltGraph' })
    vi.advanceTimersByTime(100)
    expect(sent).toEqual([
      [true, 0xfe03],
      [true, 0xfe03],
      [false, 0xfe03],
    ])
  })

  it('sends a pending Control before the next key on Windows', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    expect(sent).toEqual([])
    kb.keydown({ code: 'KeyC', key: 'c' })
    expect(sent).toEqual([
      [true, 0xffe3],
      [true, 0x63],
    ])
    kb.keyup({ code: 'KeyC', key: 'c' })
    kb.keyup({ code: 'ControlLeft', key: 'Control' })
    vi.advanceTimersByTime(100)
    expect(sent).toEqual([
      [true, 0xffe3],
      [true, 0x63],
      [false, 0x63],
      [false, 0xffe3],
    ])
  })

  it('sends a lone Control on Windows after 100 ms', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    vi.advanceTimersByTime(99)
    expect(sent).toEqual([])
    vi.advanceTimersByTime(1)
    expect(sent).toEqual([[true, 0xffe3]])
    kb.keyup({ code: 'ControlLeft', key: 'Control' })
    expect(sent).toEqual([
      [true, 0xffe3],
      [false, 0xffe3],
    ])
  })

  it('sends Control down and up when it is released before the timer', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    kb.keyup({ code: 'ControlLeft', key: 'Control' })
    vi.advanceTimersByTime(100)
    expect(sent).toEqual([
      [true, 0xffe3],
      [false, 0xffe3],
    ])
  })

  it('keeps a repeating Control pending until the first timer fires', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    vi.advanceTimersByTime(60)
    expect(kb.keydown({ code: 'ControlLeft', key: 'Control' })).toBe(true)
    expect(sent).toEqual([])
    vi.advanceTimersByTime(40)
    expect(sent).toEqual([[true, 0xffe3]])
  })

  it('sends a pending Control before a key without a keysym', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    expect(kb.keydown({ code: 'BracketLeft', key: 'Dead' })).toBe(false)
    expect(sent).toEqual([[true, 0xffe3]])
  })

  it('sends a pending Control before the keyup of another key', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'KeyA', key: 'a' })
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    kb.keyup({ code: 'KeyA', key: 'a' })
    expect(sent).toEqual([
      [true, 0x61],
      [true, 0xffe3],
      [false, 0x61],
    ])
  })

  it.each([
    ['windows', 'ControlRight', 0xffe4],
    ['other', 'ControlLeft', 0xffe3],
    ['mac', 'ControlLeft', 0xffe3],
  ] as const)('sends %s %s at once', (platform, code, keysym) => {
    const { kb, sent } = keyboard(platform)
    kb.keydown({ code, key: 'Control' })
    expect(sent).toEqual([[true, keysym]])
  })

  it('releaseAll drops a pending Control without sending it', () => {
    const { kb, sent } = keyboard('windows')
    kb.keydown({ code: 'KeyA', key: 'a' })
    kb.keydown({ code: 'ControlLeft', key: 'Control' })
    kb.releaseAll()
    vi.advanceTimersByTime(100)
    kb.keyup({ code: 'ControlLeft', key: 'Control' })
    expect(sent).toEqual([
      [true, 0x61],
      [false, 0x61],
    ])
  })

  it('releaseAll sends an up for every held key and leaves nothing held', () => {
    const { kb, sent } = keyboard('other')
    kb.keydown({ code: 'ShiftLeft', key: 'Shift' })
    kb.keydown({ code: 'KeyA', key: 'A' })
    kb.keydown({ code: 'KeyA', key: 'A' })
    kb.releaseAll()
    expect(sent.slice(3)).toEqual([
      [false, 0xffe1],
      [false, 0x41],
    ])
    kb.releaseAll()
    kb.keyup({ code: 'KeyA', key: 'A' })
    kb.keyup({ code: 'ShiftLeft', key: 'Shift' })
    expect(sent).toHaveLength(5)
  })

  it.each([
    ['MetaLeft', 0xffeb],
    ['MetaRight', 0xffec],
  ])('releases every other key before %s on macOS', (code, meta) => {
    const { kb, sent } = keyboard('mac')
    kb.keydown({ code, key: 'Meta' })
    kb.keydown({ code: 'KeyC', key: 'c' })
    expect(kb.keyup({ code, key: 'Meta' })).toBe(true)
    expect(sent).toEqual([
      [true, meta],
      [true, 0x63],
      [false, 0x63],
      [false, meta],
    ])
    kb.keydown({ code: 'KeyC', key: 'C' })
    expect(sent.slice(4)).toEqual([[true, 0x43]])
  })

  it.each(['windows', 'other'] as const)(
    'keeps other keys held when Meta is released on %s',
    (platform) => {
      const { kb, sent } = keyboard(platform)
      kb.keydown({ code: 'MetaLeft', key: 'Meta' })
      kb.keydown({ code: 'KeyC', key: 'c' })
      kb.keyup({ code: 'MetaLeft', key: 'Meta' })
      kb.keyup({ code: 'KeyC', key: 'c' })
      expect(sent).toEqual([
        [true, 0xffeb],
        [true, 0x63],
        [false, 0xffeb],
        [false, 0x63],
      ])
    },
  )
})

describe('WheelSteps', () => {
  it('turns a mouse notch into two clicks', () => {
    expect(new WheelSteps().push(0, 100, 0)).toEqual([16, 16])
    expect(new WheelSteps().push(0, -100, 0)).toEqual([8, 8])
    expect(new WheelSteps().push(120, 0, 0)).toEqual([64, 64])
    expect(new WheelSteps().push(-120, 0, 0)).toEqual([32, 32])
  })

  it('clicks once per 50 pixels of small deltas', () => {
    const wheel = new WheelSteps()
    for (let i = 0; i < 9; i++) expect(wheel.push(0, 5, 0)).toEqual([])
    expect(wheel.push(0, 5, 0)).toEqual([16])
    for (let i = 0; i < 9; i++) expect(wheel.push(0, 5, 0)).toEqual([])
    expect(wheel.push(0, 5, 0)).toEqual([16])
  })

  it('ignores a stray sub-step delta', () => {
    expect(new WheelSteps().push(0.25, 3, 0)).toEqual([])
  })

  it('keeps the remainder of a step', () => {
    const wheel = new WheelSteps()
    expect(wheel.push(0, 70, 0)).toEqual([16])
    expect(wheel.push(0, 29, 0)).toEqual([])
    expect(wheel.push(0, 1, 0)).toEqual([16])
  })

  it('counts lines as 19 pixels and pages as 800', () => {
    const lines = new WheelSteps()
    expect(lines.push(0, 3, 1)).toEqual([16])
    expect(lines.push(0, 2, 1)).toEqual([])
    expect(lines.push(0, 5, 0)).toEqual([16])
    const pages = new WheelSteps()
    expect(pages.push(0, 0.0625, 2)).toEqual([16])
    expect(pages.push(0, 49.95, 0)).toEqual([])
    expect(new WheelSteps().push(0, -0.125, 2)).toEqual([8, 8])
  })

  it('discards the remainder when the direction changes', () => {
    const wheel = new WheelSteps()
    expect(wheel.push(0, 40, 0)).toEqual([])
    expect(wheel.push(0, -40, 0)).toEqual([])
    expect(wheel.push(0, -10, 0)).toEqual([8])
  })

  it('accumulates each axis on its own', () => {
    const wheel = new WheelSteps()
    expect(wheel.push(30, 0, 0)).toEqual([])
    expect(wheel.push(0, 30, 0)).toEqual([])
    expect(wheel.push(30, 30, 0)).toEqual([16, 64])
  })

  it('yields at most 10 steps per axis and drops the rest', () => {
    const wheel = new WheelSteps()
    expect(wheel.push(-5020, 5020, 0)).toEqual([...Array(10).fill(16), ...Array(10).fill(32)])
    expect(wheel.push(0, 29, 0)).toEqual([])
    expect(wheel.push(0, 1, 0)).toEqual([16])
  })
})
