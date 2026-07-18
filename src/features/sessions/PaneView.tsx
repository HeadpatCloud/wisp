import {
  Cable,
  ChevronDown,
  ChevronUp,
  Columns2,
  FolderTree,
  Radio,
  Rows2,
  Search,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppError } from '@/bindings'
import { events } from '@/bindings'
import { Input } from '@/components/ui/input'
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
import { type TerminalSearch, TerminalView } from './TerminalView'

export function PaneView({ tabId, sessionId }: { tabId: string; sessionId: string }) {
  const session = useSessionStore((s) => s.sessions[sessionId])
  const focused = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'session' && t.activePaneId === sessionId
  })
  const tabActive = useSessionStore((s) => s.activeTabId === tabId)
  const broadcast = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'session' && !!t.broadcast
  })
  const toggleBroadcast = useSessionStore((s) => s.toggleBroadcast)
  const setZoom = useSessionStore((s) => s.setZoom)
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
  const zoom = session?.zoom ?? 0
  const settings = useMemo(() => {
    const base = appearanceFor(raw, profile?.appearance)
    return zoom ? { ...base, fontSize: Math.max(6, base.fontSize + zoom) } : base
  }, [raw, profile, zoom])
  const [connectedId, setConnectedId] = useState<string | null>(null)
  const [showSftp, setShowSftp] = useState(false)
  const [showTunnels, setShowTunnels] = useState(false)
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPrompt | null>(null)
  const [connectError, setConnectError] = useState<string | null>(null)
  const [showSearch, setShowSearch] = useState(false)
  const [query, setQuery] = useState('')
  const searchRef = useRef<TerminalSearch | null>(null)
  const termWrapRef = useRef<HTMLDivElement>(null)
  const writeRef = useRef<((data: Uint8Array) => void) | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef({ cols: 80, rows: 24 })
  const profileRef = useRef(profile)
  profileRef.current = profile
  const profileId = session?.profileId

  const bindWrite = useCallback((write: (data: Uint8Array) => void) => {
    writeRef.current = write
  }, [])
  const bindSearch = useCallback((api: TerminalSearch | null) => {
    searchRef.current = api
  }, [])
  // Only the focused pane receives keystrokes, so with broadcast on it fans them
  // out to every connected pane in the tab (itself included).
  const onData = useCallback(
    (data: string) => {
      const st = useSessionStore.getState()
      const tab = st.tabs.find((t) => t.id === tabId)
      if (tab?.kind === 'session' && tab.broadcast) {
        for (const paneId of tab.sessionIds) {
          const pane = st.sessions[paneId]
          if (pane?.sshId && pane.status === 'connected') {
            writeSession(pane.sshId, data).catch(() => {})
          }
        }
        return
      }
      if (idRef.current) writeSession(idRef.current, data).catch(() => {})
    },
    [tabId],
  )
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

  // Non-passive so Ctrl+wheel zooms this pane instead of the whole webview.
  useEffect(() => {
    const el = termWrapRef.current
    if (!el) return
    const onWheel = (ev: WheelEvent) => {
      if (!ev.ctrlKey || ev.deltaY === 0) return
      ev.preventDefault()
      const st = useSessionStore.getState()
      st.setZoom(sessionId, (st.sessions[sessionId]?.zoom ?? 0) + (ev.deltaY < 0 ? 1 : -1))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [sessionId])

  if (!session) return null

  const closeSearch = () => {
    setShowSearch(false)
    searchRef.current?.clear()
  }

  return (
    <div
      className={cn(
        'relative flex h-full w-full flex-col border',
        broadcast ? 'border-primary' : focused ? 'border-ring' : 'border-transparent',
      )}
      onMouseDownCapture={() => setActivePane(tabId, sessionId)}
      onKeyDownCapture={(ev) => {
        // Only claim Ctrl+F from the terminal itself: the SFTP panel's own inputs keep it,
        // and Shift+Ctrl+F still reaches the remote program.
        const t = ev.target as HTMLElement | null
        if (!t?.classList.contains('xterm-helper-textarea')) return
        if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && ev.key.toLowerCase() === 'f') {
          ev.preventDefault()
          ev.stopPropagation()
          setShowSearch(true)
        }
      }}
    >
      <div className="flex shrink-0 items-center justify-end gap-0.5 border-border border-b p-1">
        {zoom !== 0 && (
          <button
            type="button"
            onClick={() => setZoom(sessionId, 0)}
            className="rounded px-1 text-muted-foreground text-xs hover:bg-muted"
          >
            {zoom > 0 ? `+${zoom}` : zoom}
          </button>
        )}
        <button
          type="button"
          aria-label="Search terminal"
          onClick={() => setShowSearch((v) => !v)}
          className="rounded p-1 hover:bg-muted"
        >
          <Search className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Broadcast input to all panes"
          onClick={() => toggleBroadcast(tabId)}
          className={cn('rounded p-1 hover:bg-muted', broadcast && 'bg-muted text-primary')}
        >
          <Radio className="size-4" />
        </button>
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
        <div ref={termWrapRef} className="relative min-w-0 flex-1 bg-background p-1">
          <TerminalView
            onData={onData}
            onResize={onResize}
            bindWrite={bindWrite}
            bindSearch={bindSearch}
            settings={settings}
            copyOnSelect={raw.copyOnSelect}
            rightClickPaste={raw.rightClickPaste}
          />
          {showSearch && (
            <div className="absolute top-1 right-1 z-10 flex items-center gap-0.5 rounded border border-border bg-background p-1 shadow-md">
              <Input
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    if (e.shiftKey) searchRef.current?.findPrevious(query)
                    else searchRef.current?.findNext(query)
                  } else if (e.key === 'Escape') {
                    closeSearch()
                  }
                }}
                placeholder="Find…"
                className="h-7 w-44 text-xs"
              />
              <button
                type="button"
                aria-label="Previous match"
                onClick={() => searchRef.current?.findPrevious(query)}
                className="rounded p-1 hover:bg-muted"
              >
                <ChevronUp className="size-4" />
              </button>
              <button
                type="button"
                aria-label="Next match"
                onClick={() => searchRef.current?.findNext(query)}
                className="rounded p-1 hover:bg-muted"
              >
                <ChevronDown className="size-4" />
              </button>
              <button
                type="button"
                aria-label="Close search"
                onClick={closeSearch}
                className="rounded p-1 hover:bg-muted"
              >
                <X className="size-4" />
              </button>
            </div>
          )}
        </div>
        {showTunnels && connectedId && (
          <div className="w-72 shrink-0 border-border border-l">
            <TunnelsPanel sessionId={connectedId} profileTunnels={profile?.tunnels ?? []} />
          </div>
        )}
        {showSftp && connectedId && (
          <div className="w-96 shrink-0 border-border border-l">
            <SftpPanel
              sessionId={connectedId}
              active={tabActive && focused}
              origin={profile && { user: profile.username, host: profile.host, port: profile.port }}
            />
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
