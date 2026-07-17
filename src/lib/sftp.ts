import { Channel } from '@tauri-apps/api/core'
import { open, save } from '@tauri-apps/plugin-dialog'
import { type AuthMethod, commands, type SftpEntry, type TransferProgress } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export interface SftpAdhocParams {
  host: string
  port: number
  username: string
  authMethod: AuthMethod
  keyPath: string
  secret: string
}

// Throws the raw AppError (not unwrapped) so callers can inspect err.kind for host-key prompts.
export async function connectSftp(profileId: string): Promise<string> {
  const res = await commands.sftpConnect(profileId)
  if (res.status === 'error') throw res.error
  return res.data
}

// Throws the raw AppError, same as connectSftp. An empty password is still sent (some
// servers allow it); an empty key passphrase means an unencrypted key, so it becomes null.
export async function connectSftpAdhoc(p: SftpAdhocParams): Promise<string> {
  const res = await commands.sftpConnectAdhoc(
    p.host,
    p.port,
    p.username,
    p.authMethod,
    p.keyPath || null,
    p.authMethod === 'password' ? p.secret : p.secret || null,
  )
  if (res.status === 'error') throw res.error
  return res.data
}

export const disconnectSftp = async (sid: string): Promise<void> => {
  unwrap(await commands.sftpDisconnect(sid))
}

export const listDir = async (sid: string, path: string): Promise<SftpEntry[]> =>
  unwrap(await commands.sftpList(sid, path))

export const mkdir = async (sid: string, path: string): Promise<void> => {
  unwrap(await commands.sftpMkdir(sid, path))
}

export const rename = async (sid: string, from: string, to: string): Promise<void> => {
  unwrap(await commands.sftpRename(sid, from, to))
}

export const remove = async (sid: string, path: string, isDir: boolean): Promise<void> => {
  unwrap(await commands.sftpRemove(sid, path, isDir))
}

export const exists = async (sid: string, path: string): Promise<boolean> =>
  (await commands.sftpStat(sid, path)).status === 'ok'

export function basename(p: string): string {
  return (
    p
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() ?? p
  )
}

export async function upload(
  sid: string,
  transferId: string,
  localPath: string,
  remoteDir: string,
  onProgress: (p: TransferProgress) => void,
): Promise<string> {
  const channel = new Channel<TransferProgress>()
  channel.onmessage = onProgress
  const name = basename(localPath)
  const remotePath = remoteDir === '/' ? `/${name}` : `${remoteDir}/${name}`
  unwrap(await commands.sftpUpload(sid, transferId, localPath, remotePath, channel))
  return name
}

export async function downloadTo(
  sid: string,
  transferId: string,
  remotePath: string,
  dest: string,
  onProgress: (p: TransferProgress) => void,
): Promise<void> {
  const channel = new Channel<TransferProgress>()
  channel.onmessage = onProgress
  unwrap(await commands.sftpDownload(sid, transferId, remotePath, dest, channel))
}

export async function download(
  sid: string,
  transferId: string,
  entry: SftpEntry,
  onProgress: (p: TransferProgress) => void,
): Promise<boolean> {
  let dest: string | null
  if (entry.isDir) {
    const parent = await open({ directory: true })
    dest = typeof parent === 'string' ? `${parent}/${entry.name}` : null
  } else {
    dest = await save({ defaultPath: entry.name })
  }
  if (!dest) return false
  await downloadTo(sid, transferId, entry.path, dest, onProgress)
  return true
}

export async function cancelTransfer(transferId: string): Promise<void> {
  unwrap(await commands.sftpCancel(transferId))
}
