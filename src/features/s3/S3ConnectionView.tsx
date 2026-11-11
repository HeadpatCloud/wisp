import { useEffect, useMemo, useRef, useState } from 'react'
import { ConnectError } from '@/features/sessions/ConnectError'
import { s3View } from '@/lib/hosts'
import { connectS3, disconnectS3 } from '@/lib/s3'
import { useHostStore } from '@/stores/hostStore'
import type { SessionStatus } from '@/stores/sessionStore'
import { S3Panel } from './S3Panel'

export function S3ConnectionView({
  name,
  profileId,
  bucket,
  active,
  onStatus,
}: {
  name: string
  onStatus: (status: SessionStatus) => void
  profileId: string
  bucket: string | null
  active?: boolean
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [nonce, setNonce] = useState(0)
  const idRef = useRef<string | null>(null)
  const hosts = useHostStore((s) => s.hosts)
  const origin = useMemo(() => s3View(hosts, profileId) ?? undefined, [hosts, profileId])

  // biome-ignore lint/correctness/useExhaustiveDependencies: nonce bump retries the connect
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
  }, [profileId, nonce])

  const live = sessionId
  const status: SessionStatus = live ? 'connected' : error ? 'error' : 'connecting'
  useEffect(() => onStatus(status), [onStatus, status])

  if (sessionId)
    return <S3Panel sessionId={sessionId} bucket={bucket} active={active} origin={origin} />

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-sm">
      {error ? (
        <ConnectError name={name} reason={error} onRetry={() => setNonce((n) => n + 1)} />
      ) : (
        <p className="text-muted-foreground">Connecting…</p>
      )}
    </div>
  )
}
