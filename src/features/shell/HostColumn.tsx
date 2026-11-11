import { type ReactNode, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import { useSettingsStore } from '@/stores/settingsStore'

const MIN = 180
const MAX = 400

// The left (or, by the Host list side setting, right) column. Its inner edge drags its width,
// which is saved when the drag ends; the width is the user's number, so it goes inline.
export function HostColumn({ children }: { children: ReactNode }) {
  const saved = useSettingsStore((s) => s.settings.hostListWidth ?? 240)
  const right = useSettingsStore((s) => s.settings.hostListSide === 'right')
  const update = useSettingsStore((s) => s.update)
  const [live, setLive] = useState<number | null>(null)
  const start = useRef({ x: 0, width: saved })
  const width = live ?? saved

  return (
    <div
      className={cn('relative flex shrink-0 border-border', right ? 'border-l' : 'border-r')}
      style={{ width }}
    >
      {children}
      <button
        type="button"
        aria-label="Resize the host list"
        onKeyDown={(e) => {
          const step = e.key === 'ArrowRight' ? 10 : e.key === 'ArrowLeft' ? -10 : 0
          if (!step) return
          e.preventDefault()
          update({ hostListWidth: Math.max(MIN, Math.min(MAX, width + (right ? -step : step))) })
        }}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId)
          start.current = { x: e.clientX, width }
        }}
        onPointerMove={(e) => {
          if (!e.currentTarget.hasPointerCapture(e.pointerId)) return
          const moved = (e.clientX - start.current.x) * (right ? -1 : 1)
          setLive(Math.max(MIN, Math.min(MAX, Math.round(start.current.width + moved))))
        }}
        onPointerUp={(e) => {
          e.currentTarget.releasePointerCapture(e.pointerId)
          if (live !== null) update({ hostListWidth: live })
          setLive(null)
        }}
        className={cn(
          'absolute inset-y-0 z-10 w-1.5 cursor-col-resize focus-visible:bg-ring hover:bg-ring',
          right ? '-left-1' : '-right-1',
        )}
      />
    </div>
  )
}
