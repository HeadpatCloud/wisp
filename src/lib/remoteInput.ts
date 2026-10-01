import { LEGACY_KEYSYMS } from '@/lib/keysymTable'

const KEYSYMS = new Map([
  ['ShiftLeft', 0xffe1],
  ['ShiftRight', 0xffe2],
  ['ControlLeft', 0xffe3],
  ['ControlRight', 0xffe4],
  ['CapsLock', 0xffe5],
  ['MetaLeft', 0xffeb],
  ['MetaRight', 0xffec],
  ['AltLeft', 0xffe9],
  ['AltRight', 0xffea],
  ['Enter', 0xff0d],
  ['NumpadEnter', 0xff8d],
  ['Backspace', 0xff08],
  ['Tab', 0xff09],
  ['Escape', 0xff1b],
  ['Delete', 0xffff],
  ['Insert', 0xff63],
  ['Home', 0xff50],
  ['End', 0xff57],
  ['PageUp', 0xff55],
  ['PageDown', 0xff56],
  ['ArrowLeft', 0xff51],
  ['ArrowUp', 0xff52],
  ['ArrowRight', 0xff53],
  ['ArrowDown', 0xff54],
  ['PrintScreen', 0xff61],
  ['ScrollLock', 0xff14],
  ['Pause', 0xff13],
  ['NumLock', 0xff7f],
  ['ContextMenu', 0xff67],
])
for (let n = 1; n <= 24; n++) KEYSYMS.set(`F${n}`, 0xffbe + n - 1)

const NUMPAD_NAVIGATION = new Map([
  ['Numpad0', 0xff9e],
  ['Numpad1', 0xff9c],
  ['Numpad2', 0xff99],
  ['Numpad3', 0xff9b],
  ['Numpad4', 0xff96],
  ['Numpad5', 0xff9d],
  ['Numpad6', 0xff98],
  ['Numpad7', 0xff95],
  ['Numpad8', 0xff97],
  ['Numpad9', 0xff9a],
  ['NumpadDecimal', 0xff9f],
])

const NUMPAD_CHARACTERS = new Map([
  ['NumpadDecimal', 0xffae],
  ['NumpadAdd', 0xffab],
  ['NumpadSubtract', 0xffad],
  ['NumpadMultiply', 0xffaa],
  ['NumpadDivide', 0xffaf],
])
for (let digit = 0; digit <= 9; digit++) NUMPAD_CHARACTERS.set(`Numpad${digit}`, 0xffb0 + digit)

export function keysymFor(e: { code: string; key: string }): number | null {
  if (e.code === 'AltRight' && e.key === 'AltGraph') return 0xfe03
  const named = KEYSYMS.get(e.code)
  if (named !== undefined) return named
  if (e.key === 'Dead' || e.key === 'Process' || e.key === 'Unidentified') return null
  const cp = e.key.codePointAt(0)
  if (cp === undefined || [...e.key].length !== 1) return NUMPAD_NAVIGATION.get(e.code) ?? null
  if (cp < 0x20 || (cp >= 0x7f && cp <= 0x9f) || (cp >= 0xd800 && cp <= 0xdfff)) return null
  if (e.code === 'NumpadDecimal' && e.key === ',') return 0xffac
  const numpad = NUMPAD_CHARACTERS.get(e.code)
  if (numpad !== undefined) return numpad
  if (cp <= 0xff) return cp
  return LEGACY_KEYSYMS.get(cp) ?? 0x01000000 + cp
}

// VNC wheel buttons: 4 up, 5 down, 6 left, 7 right.
export function wheelButtons(deltaX: number, deltaY: number): number {
  let mask = 0
  if (deltaY < 0) mask |= 8
  if (deltaY > 0) mask |= 16
  if (deltaX < 0) mask |= 32
  if (deltaX > 0) mask |= 64
  return mask
}

export class HeldKeys {
  private held = new Map<string, number>()

  press(code: string, keysym: number): void {
    if (!this.held.has(code)) this.held.set(code, keysym)
  }

  release(code: string): number | null {
    const keysym = this.held.get(code)
    if (keysym === undefined) return null
    this.held.delete(code)
    return keysym
  }

  releaseAll(): number[] {
    const keysyms = [...this.held.values()]
    this.held.clear()
    return keysyms
  }
}
