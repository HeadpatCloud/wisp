import { useCallback, useEffect, useMemo, useRef } from 'react'
import { closeLocal, openLocal, resizeLocal, writeLocal } from '@/lib/local'
import { useSessionStore } from '@/stores/sessionStore'
import { appearanceOf, useSettingsStore } from '@/stores/settingsStore'
import { TerminalView } from './TerminalView'

export function LocalTerminalView({
  tabId,
  program = null,
}: {
  tabId: string
  program?: string | null
}) {
  const raw = useSettingsStore((s) => s.settings)
  const zoom = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'local' ? (t.zoom ?? 0) : 0
  })
  const settings = useMemo(() => {
    const base = appearanceOf(raw)
    return zoom ? { ...base, fontSize: Math.max(6, base.fontSize + zoom) } : base
  }, [raw, zoom])
  const wrapRef = useRef<HTMLDivElement>(null)
  const writeRef = useRef<((data: Uint8Array) => void) | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef({ cols: 80, rows: 24 })

  const bindWrite = useCallback((write: (data: Uint8Array) => void) => {
    writeRef.current = write
  }, [])
  const onData = useCallback((data: string) => {
    if (idRef.current) writeLocal(idRef.current, data)
  }, [])
  const onResize = useCallback((cols: number, rows: number) => {
    sizeRef.current = { cols, rows }
    if (idRef.current) resizeLocal(idRef.current, cols, rows)
  }, [])

  useEffect(() => {
    let disposed = false
    openLocal(program, sizeRef.current.cols, sizeRef.current.rows, (bytes) =>
      writeRef.current?.(bytes),
    )
      .then((id) => {
        if (disposed) {
          closeLocal(id)
          return
        }
        idRef.current = id
      })
      .catch(() => {})
    return () => {
      disposed = true
      if (idRef.current) closeLocal(idRef.current)
    }
  }, [program])

  // Non-passive so Ctrl+wheel zooms this tab instead of the whole webview.
  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const onWheel = (ev: WheelEvent) => {
      if (!ev.ctrlKey || ev.deltaY === 0) return
      ev.preventDefault()
      const st = useSessionStore.getState()
      const t = st.tabs.find((tab) => tab.id === tabId)
      const cur = t?.kind === 'local' ? (t.zoom ?? 0) : 0
      st.setLocalZoom(tabId, cur + (ev.deltaY < 0 ? 1 : -1))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [tabId])

  return (
    <div ref={wrapRef} className="h-full w-full bg-background p-1">
      <TerminalView onData={onData} onResize={onResize} bindWrite={bindWrite} settings={settings} />
    </div>
  )
}
