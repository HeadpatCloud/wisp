import { useEffect, useRef, useState } from 'react'
import { connectFtp, disconnectFtp } from '@/lib/ftp'
import { FtpPanel } from './FtpPanel'

export function FtpConnectionView({
  host,
  port,
  username,
  secretId,
  secure,
  allowInvalidCert,
  ignoreHostname,
  active,
}: {
  host: string
  port: number
  username: string
  secretId: string | null
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
  active?: boolean
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [nonce, setNonce] = useState(0)
  const idRef = useRef<string | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: nonce bump retries the connect
  useEffect(() => {
    let disposed = false
    idRef.current = null
    setSessionId(null)
    setError(null)
    connectFtp(host, port, username, secretId, secure, allowInvalidCert, ignoreHostname)
      .then((sid) => {
        if (disposed) {
          disconnectFtp(sid).catch(() => {})
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
      if (idRef.current) disconnectFtp(idRef.current).catch(() => {})
    }
  }, [host, port, username, secretId, secure, allowInvalidCert, ignoreHostname, nonce])

  if (sessionId)
    return (
      <FtpPanel
        sessionId={sessionId}
        active={active}
        origin={{ user: username, host, port }}
        secure={secure}
      />
    )

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-sm">
      {error ? (
        <>
          <p className="max-w-xs text-center text-destructive">{error}</p>
          <button
            type="button"
            onClick={() => setNonce((n) => n + 1)}
            className="rounded border border-border px-3 py-1.5 hover:bg-muted"
          >
            Retry
          </button>
        </>
      ) : (
        <p className="text-muted-foreground">Connecting…</p>
      )}
    </div>
  )
}
