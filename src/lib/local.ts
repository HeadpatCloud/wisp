import { Channel } from '@tauri-apps/api/core'
import { commands, type ShellInfo } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export function listShells(): Promise<ShellInfo[]> {
  return commands.listShells()
}

export async function openLocal(
  program: string | null,
  cols: number,
  rows: number,
  onBytes: (bytes: Uint8Array) => void,
): Promise<string> {
  const channel = new Channel<string>()
  channel.onmessage = (msg) => onBytes(Uint8Array.from(atob(msg), (c) => c.charCodeAt(0)))
  return unwrap(await commands.localOpen(program, cols, rows, channel))
}

export async function writeLocal(id: string, data: string): Promise<void> {
  unwrap(await commands.localWrite(id, Array.from(new TextEncoder().encode(data))))
}

export async function resizeLocal(id: string, cols: number, rows: number): Promise<void> {
  unwrap(await commands.localResize(id, cols, rows))
}

export async function closeLocal(id: string): Promise<void> {
  unwrap(await commands.localClose(id))
}

export const clearEditTemp = async (): Promise<void> => {
  unwrap(await commands.clearEditTemp())
}

export const editTempPath = async (fileName: string): Promise<string> =>
  unwrap(await commands.editTempPath(fileName))

// specta types f64 as `number | null` since non-finite floats serialize to null.
export const fileMtime = async (path: string): Promise<number> =>
  unwrap(await commands.fileMtime(path)) ?? 0
