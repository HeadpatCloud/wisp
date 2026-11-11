import { FolderTree, Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppError } from '@/bindings'
import { events } from '@/bindings'
import { sshView } from '@/lib/hosts'
import { connectErrorText } from '@/lib/ipc'
import {
  connectAdhoc,
  connectSession,
  disconnectSession,
  resizeSession,
  trustHostKey,
  writeSession,
} from '@/lib/ssh'
import { rulesFor } from '@/lib/tagRules'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { appearanceFor, useSettingsStore } from '@/stores/settingsStore'
import { useThemeStore } from '@/stores/themeStore'
import { isRunning, useTunnelStore } from '@/stores/tunnelStore'
import { startOn } from '../tunnels/run'
import { typeInto } from './broadcast'
import { ConnectError } from './ConnectError'
import { HostKeyDialog, type HostKeyPrompt } from './HostKeyDialog'
import { BroadcastButton, PaneFrame, PaneHeader, paneButton } from './PaneHeader'
import { isFindChord, TerminalSearchBox } from './TerminalSearchBox'
import { type TerminalSearch, TerminalView } from './TerminalView'
import { paneHost, pasteAsks } from './tabHosts'

export function PaneView({ tabId, paneId }: { tabId: string; paneId: string }) {
  const session = useSessionStore((s) => {
    const p = s.panes[paneId]
    return p?.kind === 'ssh' ? p : undefined
  })
  const setZoom = useSessionStore((s) => s.setZoom)
  const reconnectNonce = session?.reconnectNonce ?? 0
  const status = session?.status
  const reconnect = useSessionStore((s) => s.reconnect)
  const setStatus = useSessionStore((s) => s.setStatus)
  const setSshId = useSessionStore((s) => s.setSshId)
  const openFilesBeside = useSessionStore((s) => s.openFilesBeside)
  const hosts = useHostStore((s) => s.hosts)
  const logins = useHostStore((s) => s.logins)
  const tags = useHostStore((s) => s.tags)
  const tagColors = rulesFor(paneHost(session, hosts), tags).terminalColors
  const profile = useMemo(
    () => sshView(hosts, logins, session?.connectionId ?? undefined),
    [hosts, logins, session?.connectionId],
  )
  const raw = useSettingsStore((s) => s.settings)
  const userThemes = useThemeStore((s) => s.userThemes)
  const schemes = useThemeStore((s) => s.schemes)
  const zoom = session?.zoom ?? 0
  // biome-ignore lint/correctness/useExhaustiveDependencies: themes and schemes change what the colour ids resolve to
  const settings = useMemo(() => {
    const base = appearanceFor(raw, profile?.appearance, tagColors)
    return zoom ? { ...base, fontSize: Math.max(6, base.fontSize + zoom) } : base
  }, [raw, profile, zoom, tagColors, userThemes, schemes])
  const [connectedId, setConnectedId] = useState<string | null>(null)
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPrompt | null>(null)
  const [connectError, setConnectError] = useState<string | null>(null)
  // The fix for a key problem is in Keys & passwords, unless the login was typed for this pane.
  const [keyProblem, setKeyProblem] = useState(false)
  const [showSearch, setShowSearch] = useState(false)
  const searchRef = useRef<TerminalSearch | null>(null)
  const termWrapRef = useRef<HTMLDivElement>(null)
  const writeRef = useRef<((data: Uint8Array) => void) | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef({ cols: 80, rows: 24 })
  const tunnelCount = useTunnelStore(
    (s) =>
      Object.values(s.byId).filter(
        (t) => connectedId && t.sessionId === connectedId && isRunning(t),
      ).length,
  )
  const profileRef = useRef(profile)
  profileRef.current = profile
  const connectionId = session?.connectionId
  const adhoc = session?.adhoc

  const bindWrite = useCallback((write: (data: Uint8Array) => void) => {
    writeRef.current = write
  }, [])
  const askBeforePaste = useCallback(() => pasteAsks(paneId), [paneId])
  const bindSearch = useCallback((api: TerminalSearch | null) => {
    searchRef.current = api
  }, [])
  const onData = useCallback(
    (data: string) => {
      if (typeInto(paneId, data)) return
      if (idRef.current) writeSession(idRef.current, data).catch(() => {})
    },
    [paneId],
  )
  const onResize = useCallback((cols: number, rows: number) => {
    sizeRef.current = { cols, rows }
    if (idRef.current) resizeSession(idRef.current, cols, rows)
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: reconnectNonce triggers reconnect
  useEffect(() => {
    if (!connectionId && !adhoc) return
    idRef.current = null
    setConnectedId(null)
    setConnectError(null)
    setKeyProblem(false)
    let disposed = false
    const unlisten = events.sshStatus.listen((e) => {
      const p = e.payload
      if (idRef.current && p.sessionId === idRef.current && p.state === 'disconnected') {
        setStatus(paneId, 'closed')
        useTunnelStore.getState().clearSession(idRef.current)
      }
    })
    const onBytes = (bytes: Uint8Array) => writeRef.current?.(bytes)
    const { cols, rows } = sizeRef.current
    ;(adhoc
      ? connectAdhoc(adhoc, cols, rows, onBytes)
      : connectSession(connectionId ?? '', cols, rows, onBytes)
    )
      .then((sid) => {
        if (disposed) {
          disconnectSession(sid)
          return
        }
        idRef.current = sid
        setConnectedId(sid)
        setStatus(paneId, 'connected')
        setSshId(paneId, sid)
        for (const t of profileRef.current?.tunnels ?? []) {
          if (t.autoStart && !isRunning(useTunnelStore.getState().byId[t.id])) void startOn(sid, t)
        }
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
          setConnectError(connectErrorText(err, !adhoc))
          setKeyProblem(
            !adhoc && (err.kind === 'passphraseRequired' || err.kind === 'wrongPassphrase'),
          )
        }
        setStatus(paneId, 'error')
      })
    return () => {
      disposed = true
      unlisten.then((f) => f())
      if (idRef.current) {
        disconnectSession(idRef.current)
        useTunnelStore.getState().clearSession(idRef.current)
      }
      // A resize can still arrive before the next connect; it must not reach the closed session.
      idRef.current = null
    }
  }, [paneId, connectionId, adhoc, setStatus, setSshId, reconnectNonce])

  // Non-passive so Ctrl+wheel zooms this pane instead of the whole webview.
  useEffect(() => {
    const el = termWrapRef.current
    if (!el) return
    const onWheel = (ev: WheelEvent) => {
      if (!ev.ctrlKey || ev.deltaY === 0) return
      ev.preventDefault()
      const st = useSessionStore.getState()
      st.setZoom(paneId, (st.panes[paneId]?.zoom ?? 0) + (ev.deltaY < 0 ? 1 : -1))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [paneId])

  if (!session) return null

  const closeSearch = () => {
    setShowSearch(false)
    searchRef.current?.clear()
  }

  return (
    <PaneFrame
      tabId={tabId}
      paneId={paneId}
      onKeyDownCapture={(ev) => {
        if (!isFindChord(ev)) return
        ev.preventDefault()
        ev.stopPropagation()
        setShowSearch(true)
      }}
    >
      <PaneHeader tabId={tabId} paneId={paneId}>
        {zoom !== 0 && (
          <button
            type="button"
            onClick={() => setZoom(paneId, 0)}
            className="rounded px-1 text-muted-foreground text-xs hover:bg-muted"
          >
            {zoom > 0 ? `+${zoom}` : zoom}
          </button>
        )}
        <button
          type="button"
          aria-label="Search terminal"
          onClick={() => setShowSearch((v) => !v)}
          className={paneButton}
        >
          <Search className="size-4" />
        </button>
        <BroadcastButton tabId={tabId} />
        {tunnelCount > 0 && (
          <button
            type="button"
            onClick={() => useHostsUiStore.getState().openPage('tunnels')}
            className="rounded px-1.5 text-muted-foreground text-xs hover:bg-muted"
          >
            {tunnelCount === 1 ? '1 tunnel' : `${tunnelCount} tunnels`}
          </button>
        )}
        <button
          type="button"
          aria-label="Files"
          onClick={() => openFilesBeside(tabId, paneId)}
          disabled={!connectedId}
          className={paneButton}
        >
          <FolderTree className="size-4" />
        </button>
      </PaneHeader>
      <div className="flex min-h-0 flex-1">
        <div ref={termWrapRef} className="relative min-w-0 flex-1 bg-background p-1">
          <TerminalView
            onData={onData}
            onResize={onResize}
            bindWrite={bindWrite}
            bindSearch={bindSearch}
            settings={settings}
            copyOnSelect={raw.copyOnSelect}
            rightClickPaste={raw.rightClickPaste}
            askBeforePaste={askBeforePaste}
          />
          {showSearch && <TerminalSearchBox search={searchRef} onClose={closeSearch} />}
        </div>
      </div>
      {status === 'error' && (
        <div className="absolute inset-0 z-20 flex items-center justify-center bg-background/60">
          <ConnectError
            name={session.title}
            reason={connectError}
            onRetry={() => reconnect(paneId)}
            action={
              keyProblem
                ? {
                    label: 'Open Keys & passwords',
                    run: () => useHostsUiStore.getState().openPage('keys'),
                  }
                : undefined
            }
          />
        </div>
      )}
      {status === 'closed' && (
        <div className="absolute inset-0 z-20 flex items-center justify-center bg-background/60">
          <button
            type="button"
            onClick={() => reconnect(paneId)}
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
            reconnect(paneId)
          } catch {
            setStatus(paneId, 'error')
          }
        }}
        onReject={() => {
          setHostKeyPrompt(null)
          setStatus(paneId, 'error')
        }}
      />
    </PaneFrame>
  )
}
