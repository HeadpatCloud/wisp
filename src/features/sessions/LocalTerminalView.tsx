import { useCallback, useEffect, useMemo, useRef } from 'react'
import { closeLocal, openLocal, resizeLocal, writeLocal } from '@/lib/local'
import { appearanceOf, useSettingsStore } from '@/stores/settingsStore'
import { TerminalView } from './TerminalView'

export function LocalTerminalView({ program = null }: { program?: string | null }) {
  const raw = useSettingsStore((s) => s.settings)
  const settings = useMemo(() => appearanceOf(raw), [raw])
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

  return (
    <div className="h-full w-full bg-background p-1">
      <TerminalView onData={onData} onResize={onResize} bindWrite={bindWrite} settings={settings} />
    </div>
  )
}
