import { Channel } from '@tauri-apps/api/core'
import { commands, type SftpEntry, type TransferProgress } from '@/bindings'
import { unwrap } from '@/lib/ipc'

// Throws the raw AppError so the view can surface a connect failure message.
export async function connectS3(profileId: string): Promise<string> {
  const res = await commands.s3Connect(profileId)
  if (res.status === 'error') throw res.error
  return res.data
}

export const disconnectS3 = async (sid: string): Promise<void> => {
  unwrap(await commands.s3Disconnect(sid))
}

export const listBuckets = async (sid: string): Promise<SftpEntry[]> =>
  unwrap(await commands.s3ListBuckets(sid))

export const listObjects = async (
  sid: string,
  bucket: string,
  prefix: string,
): Promise<SftpEntry[]> => unwrap(await commands.s3List(sid, bucket, prefix))

export const mkdir = async (sid: string, bucket: string, prefix: string): Promise<void> => {
  unwrap(await commands.s3Mkdir(sid, bucket, prefix))
}

export const remove = async (
  sid: string,
  bucket: string,
  key: string,
  isDir: boolean,
): Promise<void> => {
  unwrap(await commands.s3Delete(sid, bucket, key, isDir))
}

export const rename = async (
  sid: string,
  bucket: string,
  from: string,
  to: string,
): Promise<void> => {
  unwrap(await commands.s3Rename(sid, bucket, from, to))
}

export async function upload(
  sid: string,
  bucket: string,
  key: string,
  localPath: string,
  transferId: string,
  onProgress: (p: TransferProgress) => void,
): Promise<void> {
  const channel = new Channel<TransferProgress>()
  channel.onmessage = onProgress
  unwrap(await commands.s3Upload(sid, bucket, key, localPath, transferId, channel))
}

export async function download(
  sid: string,
  bucket: string,
  key: string,
  localPath: string,
  isDir: boolean,
  size: number,
  transferId: string,
  onProgress: (p: TransferProgress) => void,
): Promise<void> {
  const channel = new Channel<TransferProgress>()
  channel.onmessage = onProgress
  unwrap(await commands.s3Download(sid, bucket, key, localPath, isDir, size, transferId, channel))
}

export const cancelS3 = async (transferId: string): Promise<void> => {
  unwrap(await commands.s3Cancel(transferId))
}
