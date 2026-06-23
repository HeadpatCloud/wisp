import { Cable, Columns2, FolderTree, Rows2, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppError } from '@/bindings'
import { events } from '@/bindings'
import {
  connectSession,
  disconnectSession,
  resizeSession,
  trustHostKey,
  writeSession,
} from '@/lib/ssh'
import { startTunnel } from '@/lib/tunnels'
import { cn } from '@/lib/utils'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { appearanceFor, useSettingsStore } from '@/stores/settingsStore'
import { useTunnelStore } from '@/stores/tunnelStore'
import { SftpPanel } from '../sftp/SftpPanel'
import { TunnelsPanel } from '../tunnels/TunnelsPanel'
import { HostKeyDialog, type HostKeyPrompt } from './HostKeyDialog'
import { TerminalView } from './TerminalView'

export function PaneView({ tabId, sessionId }: { tabId: string; sessionId: string }) {
  const session = useSessionStore((s) => s.sessions[sessionId])
  const focused = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'session' && t.activePaneId === sessionId
  })
  const tabActive = useSessionStore((s) => s.activeTabId === tabId)
  const reconnectNonce = session?.reconnectNonce ?? 0
  const status = session?.status
  const reconnect = useSessionStore((s) => s.reconnect)
  const setStatus = useSessionStore((s) => s.setStatus)
  const setSshId = useSessionStore((s) => s.setSshId)
  const setActivePane = useSessionStore((s) => s.setActivePane)
  const splitPane = useSessionStore((s) => s.splitPane)
  const closePane = useSessionStore((s) => s.closePane)
  const profile = useProfileStore((s) => s.profiles.find((p) => p.id === session?.profileId))
  const raw = useSettingsStore((s) => s.settings)
  const settings = useMemo(() => appearanceFor(raw, profile?.appearance), [raw, profile])
  const [connectedId, setConnectedId] = useState<string | null>(null)
  const [showSftp, setShowSftp] = useState(false)
  const [showTunnels, setShowTunnels] = useState(false)
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPrompt | null>(null)
  const [connectError, setConnectError] = useState<string | null>(null)
  const writeRef = useRef<((data: Uint8Array) => void) | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef({ cols: 80, rows: 24 })
  const profileRef = useRef(profile)
  profileRef.current = profile
  const profileId = session?.profileId

  const bindWrite = useCallback((write: (data: Uint8Array) => void) => {
    writeRef.current = write
  }, [])
  const onData = useCallback((data: string) => {
    if (idRef.current) writeSession(idRef.current, data)
  }, [])
  const onResize = useCallback((cols: number, rows: number) => {
    sizeRef.current = { cols, rows }
    if (idRef.current) resizeSession(idRef.current, cols, rows)
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: reconnectNonce triggers reconnect
  useEffect(() => {
    if (!profileId) return
    idRef.current = null
    setConnectedId(null)
    setConnectError(null)
    let disposed = false
    const unlisten = events.sshStatus.listen((e) => {
      const p = e.payload
      if (idRef.current && p.sessionId === idRef.current && p.state === 'disconnected') {
        setStatus(sessionId, 'closed')
        useTunnelStore.getState().clearSession(idRef.current)
      }
    })
    connectSession(profileId, sizeRef.current.cols, sizeRef.current.rows, (bytes) =>
      writeRef.current?.(bytes),
    )
      .then((sid) => {
        if (disposed) {
          disconnectSession(sid)
          return
        }
        idRef.current = sid
        setConnectedId(sid)
        setStatus(sessionId, 'connected')
        setSshId(sessionId, sid)
        profileRef.current?.tunnels
          ?.filter((t) => t.autoStart)
          .forEach((t) => {
            useTunnelStore.getState().start({
              tunnelId: t.id,
              sessionId: sid,
              state: 'starting',
              bytesUp: 0,
              bytesDown: 0,
            })
            startTunnel(sid, t).catch(() =>
              useTunnelStore
                .getState()
                .setStatus({ tunnelId: t.id, state: 'error', bytesUp: 0, bytesDown: 0 }),
            )
          })
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
          if (err.kind === 'passphraseRequired') {
            setConnectError('Key passphrase required - set it in the profile password field.')
          } else if (err.kind === 'wrongPassphrase') {
            setConnectError('Wrong key passphrase - update it in the profile password field.')
          } else if (err.kind === 'auth') {
            setConnectError(`Authentication failed: ${err.message}`)
          } else if ('message' in err && typeof err.message === 'string') {
            setConnectError(err.message)
          }
        }
        setStatus(sessionId, 'error')
      })
    return () => {
      disposed = true
      unlisten.then((f) => f())
      if (idRef.current) {
        disconnectSession(idRef.current)
        useTunnelStore.getState().clearSession(idRef.current)
      }
    }
  }, [sessionId, profileId, setStatus, setSshId, reconnectNonce])

  if (!session) return null

  return (
    <div
      className={cn(
        'relative flex h-full w-full flex-col border',
        focused ? 'border-ring' : 'border-transparent',
      )}
      onMouseDownCapture={() => setActivePane(tabId, sessionId)}
    >
      <div className="flex shrink-0 items-center justify-end gap-0.5 border-border border-b p-1">
        <button
          type="button"
          aria-label="Split right"
          onClick={() => splitPane(tabId, sessionId, 'horizontal')}
          className="rounded p-1 hover:bg-muted"
        >
          <Columns2 className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Split down"
          onClick={() => splitPane(tabId, sessionId, 'vertical')}
          className="rounded p-1 hover:bg-muted"
        >
          <Rows2 className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Toggle tunnels"
          onClick={() => setShowTunnels((v) => !v)}
          disabled={!connectedId}
          className="rounded p-1 hover:bg-muted disabled:opacity-40"
        >
          <Cable className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Toggle SFTP"
          onClick={() => setShowSftp((v) => !v)}
          disabled={!connectedId}
          className="rounded p-1 hover:bg-muted disabled:opacity-40"
        >
          <FolderTree className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Close pane"
          onClick={() => closePane(tabId, sessionId)}
          className="rounded p-1 hover:bg-muted"
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1 bg-background p-1">
          <TerminalView
            onData={onData}
            onResize={onResize}
            bindWrite={bindWrite}
            settings={settings}
          />
        </div>
        {showTunnels && connectedId && (
          <div className="w-72 shrink-0 border-border border-l">
            <TunnelsPanel sessionId={connectedId} profileTunnels={profile?.tunnels ?? []} />
          </div>
        )}
        {showSftp && connectedId && (
          <div className="w-96 shrink-0 border-border border-l">
            <SftpPanel sessionId={connectedId} active={tabActive && focused} />
          </div>
        )}
      </div>
      {(status === 'closed' || status === 'error') && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-2 bg-background/60">
          {connectError && (
            <p className="max-w-xs text-center text-destructive text-sm">{connectError}</p>
          )}
          <button
            type="button"
            onClick={() => reconnect(sessionId)}
            className="rounded border border-border px-3 py-1.5 text-sm hover:bg-muted"
          >
            Reconnect
          </button>
        </div>
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
            reconnect(sessionId)
          } catch {
            setStatus(sessionId, 'error')
          }
        }}
        onReject={() => {
          setHostKeyPrompt(null)
          setStatus(sessionId, 'error')
        }}
      />
    </div>
  )
}
