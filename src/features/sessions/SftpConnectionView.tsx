import { useEffect, useMemo, useRef, useState } from 'react'
import type { AppError } from '@/bindings'
import { sshView } from '@/lib/hosts'
import { connectErrorText } from '@/lib/ipc'
import {
  type AdhocParams,
  connectSftp,
  connectSftpAdhoc,
  connectSftpProfile,
  disconnectSftp,
} from '@/lib/sftp'
import { trustHostKey } from '@/lib/ssh'
import { useHostStore } from '@/stores/hostStore'
import type { SessionStatus } from '@/stores/sessionStore'
import { SftpPanel } from '../sftp/SftpPanel'
import { ConnectError } from './ConnectError'
import { HostKeyDialog, type HostKeyPrompt } from './HostKeyDialog'

export function SftpConnectionView({
  name,
  profileId,
  sftpProfileId,
  adhoc,
  viaSession = null,
  active,
  onStatus,
}: {
  name: string
  onStatus: (status: SessionStatus) => void
  profileId: string | null
  sftpProfileId?: string | null
  adhoc?: AdhocParams | null
  // A live terminal's SSH session: files open on it without a connection of their own.
  viaSession?: string | null
  active?: boolean
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPrompt | null>(null)
  const [nonce, setNonce] = useState(0)
  const idRef = useRef<string | null>(null)
  const hosts = useHostStore((s) => s.hosts)
  const logins = useHostStore((s) => s.logins)
  const profile = useMemo(
    () => sshView(hosts, logins, profileId ?? undefined),
    [hosts, logins, profileId],
  )
  const origin = useMemo(() => {
    if (adhoc) return { user: adhoc.username, host: adhoc.host, port: adhoc.port }
    return profile ? { user: profile.user, host: profile.host, port: profile.port } : undefined
  }, [adhoc, profile])

  // biome-ignore lint/correctness/useExhaustiveDependencies: nonce bump retries the connect
  useEffect(() => {
    let disposed = false
    idRef.current = null
    setSessionId(null)
    setError(null)
    if (viaSession) return
    const connect = adhoc
      ? connectSftpAdhoc(adhoc)
      : sftpProfileId
        ? connectSftpProfile(sftpProfileId)
        : profileId
          ? connectSftp(profileId)
          : Promise.reject(new Error('SFTP tab has no profile or connection parameters'))
    connect
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
        }
        setError(connectErrorText(e, !adhoc))
      })
    return () => {
      disposed = true
      if (idRef.current) disconnectSftp(idRef.current).catch(() => {})
    }
  }, [profileId, sftpProfileId, adhoc, viaSession, nonce])

  const live = viaSession ?? sessionId
  const status: SessionStatus = live ? 'connected' : error ? 'error' : 'connecting'
  useEffect(() => onStatus(status), [onStatus, status])

  if (live) return <SftpPanel sessionId={live} active={active} origin={origin} />

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-sm">
      {error ? (
        <ConnectError name={name} reason={error} onRetry={() => setNonce((n) => n + 1)} />
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
