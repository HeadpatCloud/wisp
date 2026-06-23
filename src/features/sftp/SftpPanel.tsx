import { useMemo } from 'react'
import { type FileBackend, FileBrowser } from '@/features/files/FileBrowser'
import { download, exists, listDir, mkdir, remove, rename, upload } from '@/lib/sftp'

export function SftpPanel({ sessionId, active }: { sessionId: string; active?: boolean }) {
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
    }),
    [sessionId],
  )
  return <FileBrowser backend={backend} active={active} />
}
