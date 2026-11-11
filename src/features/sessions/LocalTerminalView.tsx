import { Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { closeLocal, openLocal, resizeLocal, writeLocal } from '@/lib/local'
import { useSessionStore } from '@/stores/sessionStore'
import { appearanceOf, useSettingsStore } from '@/stores/settingsStore'
import { useThemeStore } from '@/stores/themeStore'
import { typeInto } from './broadcast'
import { BroadcastButton, PaneFrame, PaneHeader, paneButton } from './PaneHeader'
import { isFindChord, TerminalSearchBox } from './TerminalSearchBox'
import { type TerminalSearch, TerminalView } from './TerminalView'
import { pasteAsks } from './tabHosts'

export function LocalTerminalView({
  tabId,
  paneId,
  program = null,
  startFolder = null,
}: {
  tabId: string
  paneId: string
  program?: string | null
  startFolder?: string | null
}) {
  const raw = useSettingsStore((s) => s.settings)
  const userThemes = useThemeStore((s) => s.userThemes)
  const schemes = useThemeStore((s) => s.schemes)
  const zoom = useSessionStore((s) => s.panes[paneId]?.zoom ?? 0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: themes and schemes change what the colour ids resolve to
  const settings = useMemo(() => {
    const base = appearanceOf(raw)
    return zoom ? { ...base, fontSize: Math.max(6, base.fontSize + zoom) } : base
  }, [raw, zoom, userThemes, schemes])
  const setZoom = useSessionStore((s) => s.setZoom)
  const [showSearch, setShowSearch] = useState(false)
  const searchRef = useRef<TerminalSearch | null>(null)
  const wrapRef = useRef<HTMLDivElement>(null)
  const writeRef = useRef<((data: Uint8Array) => void) | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef({ cols: 80, rows: 24 })

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
      if (idRef.current) writeLocal(idRef.current, data)
    },
    [paneId],
  )
  const onResize = useCallback((cols: number, rows: number) => {
    sizeRef.current = { cols, rows }
    if (idRef.current) resizeLocal(idRef.current, cols, rows)
  }, [])

  useEffect(() => {
    let disposed = false
    openLocal(program, startFolder, sizeRef.current.cols, sizeRef.current.rows, (bytes) =>
      writeRef.current?.(bytes),
    )
      .then((id) => {
        if (disposed) {
          closeLocal(id)
          return
        }
        idRef.current = id
        useSessionStore.getState().setLocalId(paneId, id)
        useSessionStore.getState().setStatus(paneId, 'connected')
      })
      // The shell never started (a start folder that's gone, say): say why in the empty terminal.
      .catch((e: unknown) => {
        if (disposed) return
        useSessionStore.getState().setStatus(paneId, 'error')
        const message = e instanceof Error ? e.message : String(e)
        writeRef.current?.(new TextEncoder().encode(`${message}\r\n`))
      })
    return () => {
      disposed = true
      if (idRef.current) closeLocal(idRef.current)
      // A resize can still arrive before the next shell opens; it must not reach the closed one.
      idRef.current = null
      useSessionStore.getState().setLocalId(paneId, null)
    }
  }, [paneId, program, startFolder])

  // Non-passive so Ctrl+wheel zooms this pane instead of the whole webview.
  useEffect(() => {
    const el = wrapRef.current
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
        <BroadcastButton tabId={tabId} />
        <button
          type="button"
          aria-label="Search terminal"
          onClick={() => setShowSearch((v) => !v)}
          className={paneButton}
        >
          <Search className="size-4" />
        </button>
      </PaneHeader>
      <div ref={wrapRef} className="relative min-h-0 flex-1 bg-background p-1">
        <TerminalView
          onData={onData}
          onResize={onResize}
          bindWrite={bindWrite}
          bindSearch={bindSearch}
          settings={settings}
          askBeforePaste={askBeforePaste}
        />
        {showSearch && <TerminalSearchBox search={searchRef} onClose={closeSearch} />}
      </div>
    </PaneFrame>
  )
}
