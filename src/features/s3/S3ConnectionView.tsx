import { useEffect, useMemo, useRef, useState } from 'react'
import { connectS3, disconnectS3 } from '@/lib/s3'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { S3Panel } from './S3Panel'

export function S3ConnectionView({
  profileId,
  bucket,
  active,
}: {
  profileId: string
  bucket: string | null
  active?: boolean
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const idRef = useRef<string | null>(null)
  const profile = useS3ProfileStore((s) => s.profiles.find((p) => p.id === profileId))
  const origin = useMemo(
    () =>
      profile
        ? {
            endpoint: profile.endpoint,
            port: profile.port,
            useTls: profile.useTls,
            pathStyle: profile.pathStyle,
          }
        : undefined,
    [profile],
  )

  useEffect(() => {
    let disposed = false
    idRef.current = null
    setSessionId(null)
    setError(null)
    connectS3(profileId)
      .then((sid) => {
        if (disposed) {
          disconnectS3(sid).catch(() => {})
          return
        }
        idRef.current = sid
        setSessionId(sid)
      })
      .catch((e: unknown) => {
        if (disposed) return
        const err = e as { message?: unknown }
        setError(err && typeof err.message === 'string' ? err.message : String(e))
      })
    return () => {
      disposed = true
      if (idRef.current) disconnectS3(idRef.current).catch(() => {})
    }
  }, [profileId])

  if (sessionId)
    return <S3Panel sessionId={sessionId} bucket={bucket} active={active} origin={origin} />

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-sm">
      {error ? (
        <p className="max-w-xs text-center text-destructive">{error}</p>
      ) : (
        <p className="text-muted-foreground">Connecting...</p>
      )}
    </div>
  )
}
