import { Channel } from '@tauri-apps/api/core'
import { commands, type FrameUpdate, type VncOpened } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export async function openVnc(
  host: string,
  port: number,
  password: string,
  onFrame: (f: FrameUpdate) => void,
): Promise<VncOpened> {
  const channel = new Channel<FrameUpdate>()
  channel.onmessage = onFrame
  return unwrap(await commands.vncOpen(host, port, password, channel))
}

export async function vncPointer(id: string, buttons: number, x: number, y: number): Promise<void> {
  unwrap(await commands.vncPointer(id, buttons, x, y))
}

export async function vncKey(id: string, down: boolean, keysym: number): Promise<void> {
  unwrap(await commands.vncKey(id, down, keysym))
}

export async function vncCutText(id: string, text: string): Promise<void> {
  unwrap(await commands.vncCutText(id, text))
}

export async function vncClose(id: string): Promise<void> {
  unwrap(await commands.vncClose(id))
}

// JS MouseEvent.buttons (1=left, 2=right, 4=middle) -> VNC mask (bit0 left, bit1 middle, bit2 right).
export function vncButtonMask(jsButtons: number): number {
  let mask = 0
  if (jsButtons & 1) mask |= 1
  if (jsButtons & 4) mask |= 2
  if (jsButtons & 2) mask |= 4
  return mask
}

const KEYSYMS: Record<string, number> = {
  Enter: 0xff0d,
  Backspace: 0xff08,
  Tab: 0xff09,
  Escape: 0xff1b,
  Delete: 0xffff,
  Home: 0xff50,
  End: 0xff57,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
}

// Map a KeyboardEvent.key to an X11 keysym (null if unmapped).
export function keysymFor(key: string): number | null {
  if (key in KEYSYMS) return KEYSYMS[key]
  if (key.length === 1) return key.charCodeAt(0) // printable Latin-1 maps 1:1
  return null
}
