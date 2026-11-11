import { LayoutGrid, Plus, X } from 'lucide-react'
import { useEffect } from 'react'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { GROUP_ICON } from '@/features/hosts/icons'
import { usePointerDrag } from '@/features/hosts/usePointerDrag'
import { WindowButtons } from '@/features/shell/WindowButtons'
import { WispMenu } from '@/features/wisp/WispMenu'
import { typeGroup } from '@/lib/hosts'
import { tagColor } from '@/lib/tagColors'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import {
  type Pane,
  type SessionStatus,
  type SplitDirection,
  type Tab,
  useSessionStore,
} from '@/stores/sessionStore'
import { useTabDrag } from './tabDrag'
import { requestCloseTab, tabRules } from './tabHosts'

type TabDrop =
  | { kind: 'tab'; tabId: string; after: boolean }
  | { kind: 'edge'; tabId: string; direction: SplitDirection; before: boolean }

// A tab dropped on another tab moves there; dropped on an edge of the visible tab's
// content, it joins that tab as a row or column.
function tabAt(x: number, y: number): { target: TabDrop; el: HTMLElement } | null {
  const el = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>('[data-drop-tab],[data-drop-edge]')
  if (!el) return null
  const { dropTab, dropEdge, dropInto, dropBefore } = el.dataset
  if (dropEdge && dropInto) {
    const direction = dropEdge as SplitDirection
    return {
      target: { kind: 'edge', tabId: dropInto, direction, before: dropBefore !== undefined },
      el,
    }
  }
  if (!dropTab) return null
  const rect = el.getBoundingClientRect()
  return { target: { kind: 'tab', tabId: dropTab, after: x > rect.left + rect.width / 2 }, el }
}

function shown(t: Tab, panes: Record<string, Pane>) {
  if (t.kind !== 'panes') return { title: t.title, status: t.status, group: 'desktop' as const }
  const p = panes[t.activePaneId]
  return { title: p?.title ?? '', status: p?.status, group: p ? typeGroup(p.kind) : 'terminal' }
}

const DOT: Partial<Record<SessionStatus, string>> = {
  connecting: 'bg-muted-foreground',
  connected: 'bg-primary',
  error: 'bg-destructive',
}

// On Windows this row is the window's title bar: tabs, then a drag area and the window buttons.
export function TabBar({ titleBar, onPalette }: { titleBar: boolean; onPalette: () => void }) {
  const tabs = useSessionStore((s) => s.tabs)
  const panes = useSessionStore((s) => s.panes)
  const activeTabId = useSessionStore((s) => s.activeTabId)
  const setActiveTab = useSessionStore((s) => s.setActiveTab)
  const hosts = useHostStore((s) => s.hosts)
  const tags = useHostStore((s) => s.tags)
  const duplicateTab = useSessionStore((s) => s.duplicateTab)
  const drag = usePointerDrag<string, TabDrop>((tabId, drop) => {
    const st = useSessionStore.getState()
    if (drop.kind === 'edge') {
      st.mergeTab(tabId, drop.tabId, drop.direction, drop.before)
      return
    }
    if (tabId === drop.tabId) return
    const from = st.tabs.findIndex((t) => t.id === tabId)
    const to = st.tabs.findIndex((t) => t.id === drop.tabId) + (drop.after ? 1 : 0)
    st.moveTabTo(tabId, from < to ? to - 1 : to)
  }, tabAt)
  useEffect(() => useTabDrag.setState({ tabId: drag.dragging }), [drag.dragging])

  return (
    <div
      className={cn(
        'flex shrink-0 items-stretch border-border border-b',
        titleBar ? 'h-10' : 'h-9 compact:h-8',
      )}
    >
      <div className="flex min-w-0 items-center gap-1 overflow-x-auto px-1">
        <WispMenu />
        <div
          className={cn(
            'flex items-center rounded px-2 py-1 text-sm',
            activeTabId === null ? 'bg-accent text-accent-foreground' : 'hover:bg-muted/50',
          )}
        >
          <button
            type="button"
            onClick={() => setActiveTab(null)}
            className="flex items-center gap-1.5"
          >
            <LayoutGrid className="size-3.5" aria-hidden />
            Hosts
          </button>
        </div>
        {tabs.map((t) => {
          const { title, status, group } = shown(t, panes)
          const Icon = GROUP_ICON[group]
          const dot = status && DOT[status]
          const stripe = tagColor(tabRules(t, panes, hosts, tags).color)?.stripe
          return (
            <ContextMenu key={t.id}>
              <ContextMenuTrigger asChild>
                <div
                  data-drop-tab={t.id}
                  onPointerDown={(e) => drag.start(t.id, e)}
                  className={cn(
                    stripe && cn('border-t-2', stripe),
                    'flex items-center gap-1.5 rounded px-2 py-1 text-sm compact:py-0.5',
                    'data-drop-over:ring-2 data-drop-over:ring-ring',
                    t.id === activeTabId ? 'bg-accent text-accent-foreground' : 'hover:bg-muted/50',
                    drag.dragging === t.id && 'opacity-50',
                  )}
                >
                  <button
                    type="button"
                    onClick={() => setActiveTab(t.id)}
                    className="flex min-w-0 items-center gap-1.5"
                  >
                    <Icon className="size-3.5 shrink-0" aria-hidden />
                    <span className="max-w-40 truncate">{title}</span>
                  </button>
                  {dot && (
                    <span data-status={status} className={cn('size-1.5 rounded-full', dot)} />
                  )}
                  <button
                    type="button"
                    aria-label={`Close ${title}`}
                    onClick={() => void requestCloseTab(t.id)}
                    className="rounded p-0.5 hover:bg-muted"
                  >
                    <X className="size-3" />
                  </button>
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem onSelect={() => duplicateTab(t.id)}>Duplicate</ContextMenuItem>
                <ContextMenuItem onSelect={() => void requestCloseTab(t.id)}>Close</ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )
        })}
        <button
          type="button"
          aria-label="Command palette"
          onClick={onPalette}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <Plus className="size-4" aria-hidden />
        </button>
      </div>
      <div data-tauri-drag-region className="min-w-8 flex-1" />
      {titleBar && <WindowButtons />}
    </div>
  )
}
