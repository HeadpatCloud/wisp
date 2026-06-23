import { Channel } from '@tauri-apps/api/core'
import { commands } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export async function connectSession(
  profileId: string,
  cols: number,
  rows: number,
  onBytes: (bytes: Uint8Array) => void,
): Promise<string> {
  // Rust sends base64(bytes) over Channel<String>. (tauri::ipc::Response is not
  // specta-typeable, and Channel<Vec<u8>> would JSON-encode each byte as a number.)
  // Decode base64 -> bytes.
  const channel = new Channel<string>()
  channel.onmessage = (msg) => onBytes(Uint8Array.from(atob(msg), (c) => c.charCodeAt(0)))
  const res = await commands.sshConnect(profileId, cols, rows, channel)
  if (res.status === 'error') throw res.error
  return res.data
}

export async function trustHostKey(host: string, port: number, fingerprint: string): Promise<void> {
  unwrap(await commands.trustHostKey(host, port, fingerprint))
}

export async function writeSession(id: string, data: string): Promise<void> {
  unwrap(await commands.sshWrite(id, Array.from(new TextEncoder().encode(data))))
}

export async function resizeSession(id: string, cols: number, rows: number): Promise<void> {
  unwrap(await commands.sshResize(id, cols, rows))
}

export async function disconnectSession(id: string): Promise<void> {
  unwrap(await commands.sshDisconnect(id))
}
