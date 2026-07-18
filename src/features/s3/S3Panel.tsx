import { open, save } from '@tauri-apps/plugin-dialog'
import { useMemo } from 'react'
import { type FileBackend, FileBrowser } from '@/features/files/FileBrowser'
import * as s3 from '@/lib/s3'
import { basename } from '@/lib/sftp'
import { encodePath } from '@/lib/urls'

// FileBrowser paths are "/bucket/key...". '.', '/' and '' all mean root (the bucket list).
function isRoot(path: string): boolean {
  return path === '' || path === '.' || path === '/'
}
function split(path: string): { bucket: string; key: string } {
  const t = path.replace(/^\/+/, '')
  const i = t.indexOf('/')
  return i === -1 ? { bucket: t, key: '' } : { bucket: t.slice(0, i), key: t.slice(i + 1) }
}

export interface S3Origin {
  endpoint: string
  port: number | null
  useTls: boolean
  pathStyle: boolean
}

function publicUrl(o: S3Origin, bucket: string, key: string): string {
  const scheme = o.useTls ? 'https' : 'http'
  const defaultPort = o.useTls ? 443 : 80
  const authority = o.port && o.port !== defaultPort ? `${o.endpoint}:${o.port}` : o.endpoint
  return o.pathStyle
    ? `${scheme}://${authority}/${bucket}/${encodePath(key)}`
    : `${scheme}://${bucket}.${authority}/${encodePath(key)}`
}

export function S3Panel({
  sessionId,
  bucket,
  active,
  origin,
}: {
  sessionId: string
  bucket: string | null
  active?: boolean
  origin?: S3Origin
}) {
  const backend = useMemo<FileBackend>(
    () => ({
      list: async (path) => {
        if (isRoot(path)) {
          if (bucket) {
            const entries = await s3.listObjects(sessionId, bucket, '')
            return entries.map((e) => ({ ...e, path: `/${bucket}/${e.path}` }))
          }
          const buckets = await s3.listBuckets(sessionId)
          return buckets.map((e) => ({ ...e, path: `/${e.path}` }))
        }
        const { bucket: b, key } = split(path)
        const entries = await s3.listObjects(sessionId, b, key)
        return entries.map((e) => ({ ...e, path: `/${b}/${e.path}` }))
      },
      // S3 PUT overwrites by default, so there's no conflict to confirm.
      exists: async () => false,
      mkdir: async (path) => {
        const { bucket: b, key } = split(path)
        if (!b || !key) throw new Error('Open a bucket first')
        await s3.mkdir(sessionId, b, key)
      },
      rename: async (from, to) => {
        const a = split(from)
        if (a.key.endsWith('/')) throw new Error("Renaming folders isn't supported on S3")
        await s3.rename(sessionId, a.bucket, a.key, split(to).key)
      },
      remove: async (path, isDir) => {
        const { bucket: b, key } = split(path)
        if (!key) throw new Error("Deleting a bucket isn't supported here")
        await s3.remove(sessionId, b, key, isDir)
      },
      upload: async (id, localPath, remoteDir, onProgress) => {
        const { bucket: b, key } = split(remoteDir)
        if (!b) throw new Error('Open a bucket first')
        const prefix = key && !key.endsWith('/') ? `${key}/` : key
        const name = basename(localPath)
        await s3.upload(sessionId, b, `${prefix}${name}`, localPath, id, onProgress)
        return name
      },
      download: async (id, entry, onProgress) => {
        const { bucket: b, key } = split(entry.path)
        let dest: string | null
        if (entry.isDir) {
          const parent = await open({ directory: true })
          dest = typeof parent === 'string' ? `${parent}/${entry.name}` : null
        } else {
          dest = await save({ defaultPath: entry.name })
        }
        if (!dest) return false
        await s3.download(sessionId, b, key, dest, entry.isDir, entry.size, id, onProgress)
        return true
      },
      downloadTo: async (id, entry, dest, onProgress) => {
        const { bucket: b, key } = split(entry.path)
        await s3.download(sessionId, b, key, dest, entry.isDir, entry.size, id, onProgress)
      },
      links: origin && {
        url: (entry) => {
          const { bucket: b, key } = split(entry.path)
          return publicUrl(origin, b, key)
        },
        signedUrl: (entry, expiresSecs) => {
          const { bucket: b, key } = split(entry.path)
          return s3.presign(sessionId, b, key, expiresSecs)
        },
      },
    }),
    [sessionId, bucket, origin],
  )

  return <FileBrowser backend={backend} active={active} initialPath={bucket ? `/${bucket}` : '/'} />
}
