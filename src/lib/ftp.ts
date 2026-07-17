import { Channel } from '@tauri-apps/api/core'
import { open, save } from '@tauri-apps/plugin-dialog'
import { commands, type SftpEntry, type TransferProgress } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import { basename } from '@/lib/sftp'

// Throws the raw AppError so callers can surface a connect failure message.
export async function connectFtp(
  host: string,
  port: number,
  username: string,
  password: string,
  secure: boolean,
  allowInvalidCert: boolean,
  ignoreHostname: boolean,
): Promise<string> {
  const res = await commands.ftpConnect(
    host,
    port,
    username,
    password,
    secure,
    allowInvalidCert,
    ignoreHostname,
  )
  if (res.status === 'error') throw res.error
  return res.data
}

export const disconnectFtp = async (sid: string): Promise<void> => {
  unwrap(await commands.ftpDisconnect(sid))
}

export const cancelFtp = async (transferId: string): Promise<void> => {
  unwrap(await commands.ftpCancel(transferId))
}

export const listDir = async (sid: string, path: string): Promise<SftpEntry[]> =>
  unwrap(await commands.ftpList(sid, path))

export const exists = async (sid: string, path: string): Promise<boolean> =>
  unwrap(await commands.ftpExists(sid, path))

export const mkdir = async (sid: string, path: string): Promise<void> => {
  unwrap(await commands.ftpMkdir(sid, path))
}

export const rename = async (sid: string, from: string, to: string): Promise<void> => {
  unwrap(await commands.ftpRename(sid, from, to))
}

export const remove = async (sid: string, path: string, isDir: boolean): Promise<void> => {
  unwrap(await commands.ftpRemove(sid, path, isDir))
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
  unwrap(await commands.ftpUpload(sid, transferId, localPath, remotePath, channel))
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
  unwrap(await commands.ftpDownload(sid, transferId, remotePath, dest, channel))
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
