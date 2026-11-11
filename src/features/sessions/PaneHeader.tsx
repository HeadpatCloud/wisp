import { Columns2, Radio, Rows2, X } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { useSessionStore } from '@/stores/sessionStore'
import { requestClosePane } from './tabHosts'

export const paneButton = 'rounded p-1 hover:bg-muted disabled:opacity-40'

export function PaneHeader({
  tabId,
  paneId,
  children,
}: {
  tabId: string
  paneId: string
  children?: ReactNode
}) {
  const splitPane = useSessionStore((s) => s.splitPane)
  return (
    <div className="flex shrink-0 items-center justify-end gap-0.5 border-border border-b p-1">
      {children}
      <button
        type="button"
        aria-label="Split right"
        onClick={() => splitPane(tabId, paneId, 'horizontal')}
        className={paneButton}
      >
        <Columns2 className="size-4" />
      </button>
      <button
        type="button"
        aria-label="Split down"
        onClick={() => splitPane(tabId, paneId, 'vertical')}
        className={paneButton}
      >
        <Rows2 className="size-4" />
      </button>
      <button
        type="button"
        aria-label="Close pane"
        onClick={() => void requestClosePane(tabId, paneId)}
        className={paneButton}
      >
        <X className="size-4" />
      </button>
    </div>
  )
}

export function BroadcastButton({ tabId }: { tabId: string }) {
  const toggleBroadcast = useSessionStore((s) => s.toggleBroadcast)
  const on = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'panes' && !!t.broadcast
  })
  return (
    <button
      type="button"
      aria-label="Type into every pane of the tab"
      aria-pressed={on}
      onClick={() => toggleBroadcast(tabId)}
      className={cn(paneButton, on && 'bg-muted text-primary')}
    >
      <Radio className="size-4" />
    </button>
  )
}

// The pane's outline shows which pane has focus, and that typing goes to every pane.
export function PaneFrame({
  tabId,
  paneId,
  className,
  onKeyDownCapture,
  children,
}: {
  tabId: string
  paneId: string
  className?: string
  onKeyDownCapture?: (ev: React.KeyboardEvent) => void
  children: ReactNode
}) {
  const setActivePane = useSessionStore((s) => s.setActivePane)
  const focused = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'panes' && t.activePaneId === paneId
  })
  const broadcast = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return t?.kind === 'panes' && !!t.broadcast
  })
  const terminal = useSessionStore((s) => {
    const kind = s.panes[paneId]?.kind
    return kind === 'ssh' || kind === 'local'
  })
  return (
    <div
      className={cn(
        'relative flex h-full w-full flex-col border',
        broadcast && terminal ? 'border-primary' : focused ? 'border-ring' : 'border-transparent',
        className,
      )}
      onMouseDownCapture={() => setActivePane(tabId, paneId)}
      onKeyDownCapture={onKeyDownCapture}
    >
      {children}
    </div>
  )
}
