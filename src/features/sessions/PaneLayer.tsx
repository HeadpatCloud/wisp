import { useEffect, useLayoutEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { useSessionStore } from '@/stores/sessionStore'
import { FilesPane } from './FilesPane'
import { LocalTerminalView } from './LocalTerminalView'
import { PaneView } from './PaneView'

// Each pane renders into an element of its own, which the slot of whatever tab holds the pane
// adopts. Moving a pane to another tab then moves only that element: the pane's component, its
// terminal and its session live on.
const hosts = new Map<string, HTMLDivElement>()

function hostFor(paneId: string): HTMLDivElement {
  let el = hosts.get(paneId)
  if (!el) {
    el = document.createElement('div')
    el.className = 'h-full w-full'
    hosts.set(paneId, el)
  }
  return el
}

function PaneByKind({ tabId, paneId }: { tabId: string; paneId: string }) {
  const pane = useSessionStore((s) => s.panes[paneId])
  const active = useSessionStore((s) => {
    const t = s.tabs.find((tab) => tab.id === tabId)
    return s.activeTabId === tabId && t?.kind === 'panes' && t.activePaneId === paneId
  })
  if (!pane) return null
  switch (pane.kind) {
    case 'ssh':
      return <PaneView tabId={tabId} paneId={paneId} />
    case 'local':
      return (
        <LocalTerminalView
          tabId={tabId}
          paneId={paneId}
          program={pane.program}
          startFolder={pane.startFolder}
        />
      )
    default:
      return <FilesPane tabId={tabId} pane={pane} active={active} />
  }
}

export function PaneLayer() {
  const tabs = useSessionStore((s) => s.tabs)
  // In a fixed order, so reordering tabs never moves a pane's component (StrictMode would
  // re-run its effects for a move, and reconnect it).
  const placed = tabs
    .flatMap((t) =>
      t.kind === 'panes' ? t.paneIds.map((paneId) => ({ tabId: t.id, paneId })) : [],
    )
    .sort((a, b) => a.paneId.localeCompare(b.paneId))
  const ids = placed.map((p) => p.paneId).join(' ')

  // A closed pane's element goes once its component has unmounted.
  useEffect(() => {
    const live = new Set(ids.split(' '))
    for (const [id, el] of hosts) {
      if (live.has(id)) continue
      el.remove()
      hosts.delete(id)
    }
  }, [ids])

  return placed.map(({ tabId, paneId }) =>
    createPortal(<PaneByKind tabId={tabId} paneId={paneId} />, hostFor(paneId), paneId),
  )
}

export function PaneSlot({ paneId }: { paneId: string }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const slot = ref.current
    const el = hostFor(paneId)
    slot?.appendChild(el)
    return () => {
      if (el.parentElement === slot) el.remove()
    }
  }, [paneId])
  return <div ref={ref} className="h-full w-full" />
}
