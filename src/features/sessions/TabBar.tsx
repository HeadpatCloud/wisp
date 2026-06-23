import { X } from 'lucide-react'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { cn } from '@/lib/utils'
import { useSessionStore } from '@/stores/sessionStore'

export function TabBar() {
  const tabs = useSessionStore((s) => s.tabs)
  const sessions = useSessionStore((s) => s.sessions)
  const activeTabId = useSessionStore((s) => s.activeTabId)
  const setActiveTab = useSessionStore((s) => s.setActiveTab)
  const removeTab = useSessionStore((s) => s.removeTab)
  const duplicateTab = useSessionStore((s) => s.duplicateTab)

  if (tabs.length === 0) return null

  return (
    <div className="flex h-9 shrink-0 items-center gap-1 border-border border-b px-1">
      {tabs.map((t) => {
        const isSession = t.kind === 'session'
        const title = isSession ? (sessions[t.activePaneId]?.title ?? 'session') : t.title
        const status = isSession ? sessions[t.activePaneId]?.status : undefined
        const canDuplicate = t.kind === 'session' || t.kind === 'local'
        return (
          <ContextMenu key={t.id}>
            <ContextMenuTrigger asChild>
              <div
                className={cn(
                  'flex items-center gap-1.5 rounded px-2 py-1 text-sm',
                  t.id === activeTabId ? 'bg-accent text-accent-foreground' : 'hover:bg-muted/50',
                )}
              >
                <button
                  type="button"
                  onClick={() => setActiveTab(t.id)}
                  className="max-w-40 truncate"
                >
                  {title}
                </button>
                {isSession && (
                  <span
                    className={cn(
                      'size-1.5 rounded-full',
                      status === 'connected' && 'bg-green-500',
                      status === 'connecting' && 'bg-yellow-500',
                      (status === 'closed' || status === 'error') && 'bg-red-500',
                    )}
                  />
                )}
                <button
                  type="button"
                  aria-label={`Close ${title}`}
                  onClick={() => removeTab(t.id)}
                  className="rounded p-0.5 hover:bg-muted"
                >
                  <X className="size-3" />
                </button>
              </div>
            </ContextMenuTrigger>
            {canDuplicate && (
              <ContextMenuContent>
                <ContextMenuItem onSelect={() => duplicateTab(t.id)}>Duplicate</ContextMenuItem>
              </ContextMenuContent>
            )}
          </ContextMenu>
        )
      })}
    </div>
  )
}
