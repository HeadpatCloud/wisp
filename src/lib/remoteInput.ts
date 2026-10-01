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

const MODIFIERS = new Set([
  'ShiftLeft',
  'ShiftRight',
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'MetaLeft',
  'MetaRight',
])

// Virtual keyboards and input injectors report keys without a usable code.
function hasNoCode(code: string): boolean {
  return code === '' || code === 'Unidentified'
}

export function keysymFor(e: { code: string; key: string }): number | null {
  if (e.code === 'AltRight' && e.key === 'AltGraph') return 0xfe03
  const named = KEYSYMS.get(hasNoCode(e.code) ? e.key : e.code)
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

// DOM buttons (1 left, 2 right, 4 middle) to the VNC mask (bit 0 left, bit 1 middle, bit 2 right).
export function pointerButtons(domButtons: number): number {
  let mask = 0
  if (domButtons & 1) mask |= 1
  if (domButtons & 4) mask |= 2
  if (domButtons & 2) mask |= 4
  return mask
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

export class WheelSteps {
  private x = 0
  private y = 0

  // One step is 50 pixels; a line counts as 19 pixels and a page as 800.
  push(deltaX: number, deltaY: number, deltaMode: number): number[] {
    const scale = deltaMode === 1 ? 19 : deltaMode === 2 ? 800 : 1
    const stepsY = this.take('y', deltaY * scale)
    const stepsX = this.take('x', deltaX * scale)
    return [
      ...Array<number>(Math.min(Math.abs(stepsY), 10)).fill(wheelButtons(0, stepsY)),
      ...Array<number>(Math.min(Math.abs(stepsX), 10)).fill(wheelButtons(stepsX, 0)),
    ]
  }

  private take(axis: 'x' | 'y', delta: number): number {
    if (!Number.isFinite(delta)) return 0
    const total = (this[axis] * delta < 0 ? 0 : this[axis]) + delta
    this[axis] = total % 50
    return Math.trunc(total / 50)
  }
}

export class HeldKeys {
  private held = new Map<string, number>()

  press(code: string, keysym: number): number {
    const held = this.held.get(code)
    if (held !== undefined) return held
    this.held.set(code, keysym)
    return keysym
  }

  has(code: string): boolean {
    return this.held.has(code)
  }

  codes(): string[] {
    return [...this.held.keys()]
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

export type Platform = 'windows' | 'mac' | 'other'

export function detectPlatform(userAgent: string): Platform {
  if (userAgent.includes('Windows')) return 'windows'
  if (userAgent.includes('Mac')) return 'mac'
  return 'other'
}

export class RemoteKeyboard {
  private held = new HeldKeys()
  private pendingControl: {
    keysym: number
    timeStamp?: number
    timer: ReturnType<typeof setTimeout>
  } | null = null

  constructor(
    private send: (down: boolean, keysym: number) => void,
    private platform: Platform,
  ) {}

  keydown(e: { code: string; key: string; timeStamp?: number; metaKey?: boolean }): boolean {
    const pending = this.pendingControl
    if (pending) {
      if (e.code === 'ControlLeft') return true
      // Windows reports AltGr as ControlLeft followed at once by AltRight; that Control is not sent.
      const altGr =
        e.code === 'AltRight' &&
        e.key === 'AltGraph' &&
        (e.timeStamp === undefined ||
          pending.timeStamp === undefined ||
          e.timeStamp - pending.timeStamp < 50)
      if (altGr) this.cancelControl()
      else this.flush()
    }
    const keysym = keysymFor(e)
    if (keysym === null) return false
    // macOS reports no keyup for a non-modifier key pressed while Cmd is down.
    const underCmd =
      this.platform === 'mac' &&
      !MODIFIERS.has(e.code) &&
      !this.held.has(e.code) &&
      (e.metaKey === true || this.held.has('MetaLeft') || this.held.has('MetaRight'))
    if (hasNoCode(e.code) || this.macCapsLock(e.code) || underCmd) {
      this.click(keysym)
    } else if (
      this.platform === 'windows' &&
      e.code === 'ControlLeft' &&
      !this.held.has('ControlLeft')
    ) {
      const timer = setTimeout(() => this.flush(), 100)
      this.pendingControl = { keysym, timeStamp: e.timeStamp, timer }
    } else {
      this.send(true, this.held.press(e.code, keysym))
    }
    return true
  }

  keyup(e: { code: string; key: string; timeStamp?: number; metaKey?: boolean }): boolean {
    this.flush()
    const mapped = keysymFor(e)
    if (mapped !== null && this.macCapsLock(e.code)) {
      this.click(mapped)
      return true
    }
    // macOS never reports the keyup of a non-modifier key let go while Cmd was down, so those
    // keys go up with the last Cmd.
    if (
      this.platform === 'mac' &&
      (e.code === 'MetaLeft' || e.code === 'MetaRight') &&
      !this.held.has(e.code === 'MetaLeft' ? 'MetaRight' : 'MetaLeft')
    ) {
      for (const code of this.held.codes()) {
        if (!MODIFIERS.has(code)) this.up(code)
      }
    }
    const wasHeld = this.up(e.code)
    // Windows delivers a single keyup when both Shift keys were down.
    if (this.platform === 'windows' && (e.code === 'ShiftLeft' || e.code === 'ShiftRight')) {
      this.up(e.code === 'ShiftLeft' ? 'ShiftRight' : 'ShiftLeft')
    }
    return wasHeld || mapped !== null
  }

  flush(): void {
    const pending = this.pendingControl
    if (!pending) return
    this.cancelControl()
    this.send(true, this.held.press('ControlLeft', pending.keysym))
  }

  releaseAll(): void {
    this.cancelControl()
    for (const keysym of this.held.releaseAll()) this.send(false, keysym)
  }

  private click(keysym: number): void {
    this.send(true, keysym)
    this.send(false, keysym)
  }

  private up(code: string): boolean {
    const keysym = this.held.release(code)
    if (keysym === null) return false
    this.send(false, keysym)
    return true
  }

  // macOS reports Caps Lock turning on as a keydown and turning off as a keyup.
  private macCapsLock(code: string): boolean {
    return this.platform === 'mac' && code === 'CapsLock'
  }

  private cancelControl(): void {
    clearTimeout(this.pendingControl?.timer)
    this.pendingControl = null
  }
}
