import { useEffect, useRef, useState } from 'react'
import type { AppError } from '@/bindings'
import { connectSftp, disconnectSftp } from '@/lib/sftp'
import { trustHostKey } from '@/lib/ssh'
import { SftpPanel } from '../sftp/SftpPanel'
import { HostKeyDialog, type HostKeyPrompt } from './HostKeyDialog'

export function SftpConnectionView({ profileId, active }: { profileId: string; active?: boolean }) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPrompt | null>(null)
  const [nonce, setNonce] = useState(0)
  const idRef = useRef<string | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: nonce bump retries the connect
  useEffect(() => {
    let disposed = false
    idRef.current = null
    setSessionId(null)
    setError(null)
    connectSftp(profileId)
      .then((sid) => {
        if (disposed) {
          disconnectSftp(sid).catch(() => {})
          return
        }
        idRef.current = sid
        setSessionId(sid)
      })
      .catch((e: unknown) => {
        if (disposed) return
        const err = e as AppError
        if (err && typeof err === 'object' && 'kind' in err) {
          if (err.kind === 'hostKeyUnknown') {
            setHostKeyPrompt({ kind: 'unknown', ...err.message })
            return
          }
          if (err.kind === 'hostKeyMismatch') {
            const { host, port, stored, offered } = err.message
            setHostKeyPrompt({ kind: 'mismatch', host, port, stored, offered })
            return
          }
          if ('message' in err && typeof err.message === 'string') {
            setError(err.message)
            return
          }
        }
        setError(String(e))
      })
    return () => {
      disposed = true
      if (idRef.current) disconnectSftp(idRef.current).catch(() => {})
    }
  }, [profileId, nonce])

  if (sessionId) return <SftpPanel sessionId={sessionId} active={active} />

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
      ) : hostKeyPrompt ? null : (
        <p className="text-muted-foreground">Connecting…</p>
      )}
      <HostKeyDialog
        prompt={hostKeyPrompt}
        onAccept={async () => {
          const p = hostKeyPrompt
          setHostKeyPrompt(null)
          if (!p) return
          const fingerprint = p.kind === 'unknown' ? p.fingerprint : p.offered
          try {
            await trustHostKey(p.host, p.port, fingerprint)
            setNonce((n) => n + 1)
          } catch {
            setError('Failed to trust host key.')
          }
        }}
        onReject={() => {
          setHostKeyPrompt(null)
          setError('Host key rejected.')
        }}
      />
    </div>
  )
}
