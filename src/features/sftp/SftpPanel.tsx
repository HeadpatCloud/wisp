import { useMemo } from 'react'
import { type FileBackend, FileBrowser } from '@/features/files/FileBrowser'
import { download, downloadTo, exists, listDir, mkdir, remove, rename, upload } from '@/lib/sftp'
import { type RemoteOrigin, remoteUrl } from '@/lib/urls'

export function SftpPanel({
  sessionId,
  active,
  origin,
}: {
  sessionId: string
  active?: boolean
  origin?: RemoteOrigin
}) {
  const backend = useMemo<FileBackend>(
    () => ({
      list: (path) => listDir(sessionId, path),
      exists: (path) => exists(sessionId, path),
      mkdir: (path) => mkdir(sessionId, path),
      rename: (from, to) => rename(sessionId, from, to),
      remove: (path, isDir) => remove(sessionId, path, isDir),
      upload: (id, localPath, remoteDir, onProgress) =>
        upload(sessionId, id, localPath, remoteDir, onProgress),
      download: (id, entry, onProgress) => download(sessionId, id, entry, onProgress),
      downloadTo: (id, entry, dest, onProgress) =>
        downloadTo(sessionId, id, entry.path, dest, onProgress),
      links: origin && { url: (entry) => remoteUrl('sftp', origin, entry.path) },
    }),
    [sessionId, origin],
  )
  return <FileBrowser backend={backend} active={active} />
}
