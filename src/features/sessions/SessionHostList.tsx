import {
  ChevronDown,
  ChevronRight,
  LayoutGrid,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Settings,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Host } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { Input } from '@/components/ui/input'
import { GROUP_ICON } from '@/features/hosts/icons'
import { openConnection } from '@/features/hosts/openConnection'
import { ProfileIcon } from '@/features/hosts/ProfileIcon'
import {
  connectionLabel,
  folderOrder,
  folderPath,
  hostMatches,
  THIS_COMPUTER,
  typeGroup,
} from '@/lib/hosts'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { HostColumn } from '../shell/HostColumn'

const strip = 'flex size-8 items-center justify-center rounded-md hover:bg-muted'

function openMain(h: Host) {
  const main = h.connections[0]
  if (main) openConnection(h, main)
}

// Connection ids that have a tab or pane open, so the strip can mark their hosts.
function useOpenConnections(): Set<string> {
  const tabs = useSessionStore((s) => s.tabs)
  const panes = useSessionStore((s) => s.panes)
  return useMemo(() => {
    const ids = new Set<string>()
    for (const t of tabs) {
      if (t.kind !== 'panes') {
        if (t.connectionId) ids.add(t.connectionId)
        continue
      }
      for (const id of t.paneIds) {
        const p = panes[id]
        if (p?.connectionId) ids.add(p.connectionId)
        if (p?.kind === 'sftp' && p.sshConnectionId) ids.add(p.sshConnectionId)
      }
    }
    return ids
  }, [tabs, panes])
}

function IconStrip({ hosts }: { hosts: Host[] }) {
  const toggle = useHostsUiStore((s) => s.toggleHostList)
  const openPage = useHostsUiStore((s) => s.openPage)
  const openNewConnection = useHostsUiStore((s) => s.openNewConnection)
  const open = useOpenConnections()
  const right = useSettingsStore((s) => s.settings.hostListSide === 'right')
  return (
    <nav
      aria-label="Hosts"
      className={cn(
        'flex w-12 shrink-0 flex-col items-center gap-1 overflow-y-auto border-border bg-sidebar py-2',
        right ? 'border-l' : 'border-r',
      )}
    >
      <button type="button" aria-label="Show the host list" onClick={toggle} className={strip}>
        <PanelLeftOpen className="size-4" aria-hidden />
      </button>
      <button
        type="button"
        aria-label="New connection"
        onClick={() => openNewConnection()}
        className={strip}
      >
        <Plus className="size-4" aria-hidden />
      </button>
      <button type="button" aria-label="Hosts" onClick={() => openPage('hosts')} className={strip}>
        <LayoutGrid className="size-4" aria-hidden />
      </button>
      <div className="my-1 h-px w-6 bg-border" />
      {hosts.map((h) => (
        <button
          key={h.id}
          type="button"
          aria-label={h.name}
          title={h.name}
          onClick={() => openMain(h)}
          className={cn(strip, 'relative')}
        >
          <ProfileIcon icon={h.icon} className="size-4 text-muted-foreground" />
          {h.connections.some((c) => open.has(c.id)) && (
            <span className="absolute right-1 bottom-1 size-1.5 rounded-full bg-primary" />
          )}
        </button>
      ))}
      <button
        type="button"
        aria-label="Settings"
        onClick={() => openPage('settings')}
        className={cn(strip, 'mt-auto')}
      >
        <Settings className="size-4" aria-hidden />
      </button>
    </nav>
  )
}

export function SessionHostList() {
  const collapsed = useHostsUiStore((s) => s.hostListCollapsed)
  const toggle = useHostsUiStore((s) => s.toggleHostList)
  const hosts = useHostStore((s) => s.hosts)
  const folders = useHostStore((s) => s.folders)
  const tags = useHostStore((s) => s.tags)
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  const [query, setQuery] = useState('')
  const [unfolded, setUnfolded] = useState<Set<string>>(new Set())

  const pc = hosts.find((h) => h.id === THIS_COMPUTER)
  const others = hosts.filter((h) => h.id !== THIS_COMPUTER)
  const byOrder = (list: Host[]) => [...list].sort((a, b) => a.order - b.order)
  const order = folderOrder(folders)
  const groups = order
    .map((id) => ({
      id,
      title: folderPath(id, folders),
      hosts: byOrder(others.filter((h) => h.folderId === id)),
    }))
    .filter((g) => g.hosts.length > 0)
  const loose = byOrder(others.filter((h) => !h.folderId || !order.includes(h.folderId)))
  if (loose.length > 0) groups.push({ id: '', title: '', hosts: loose })
  const all = [...(pc ? [pc] : []), ...groups.flatMap((g) => g.hosts)]

  if (collapsed) return <IconStrip hosts={all} />

  const shown = (h: Host) => hostMatches(h, query, tags)
  const fold = (id: string) =>
    setUnfolded((s) => {
      const next = new Set(s)
      if (!next.delete(id)) next.add(id)
      return next
    })

  const hostRow = (h: Host) => {
    const open = query !== '' || unfolded.has(h.id)
    return (
      <li key={h.id}>
        <button
          type="button"
          onClick={() => fold(h.id)}
          onDoubleClick={() => openMain(h)}
          onKeyDown={(e) => {
            if (e.key !== 'Enter') return
            e.preventDefault()
            openMain(h)
          }}
          aria-expanded={open}
          className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left compact:py-0.5 text-sm hover:bg-muted"
        >
          {open ? (
            <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          ) : (
            <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          )}
          <ProfileIcon icon={h.icon} className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate">{h.name}</span>
        </button>
        {open && (
          <ul className="pl-6">
            {h.connections.map((c) => {
              const Icon = GROUP_ICON[typeGroup(c.kind.type)]
              return (
                <li key={c.id}>
                  <ContextMenu>
                    <ContextMenuTrigger asChild>
                      <button
                        type="button"
                        onClick={() => openConnection(h, c)}
                        className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left compact:py-0.5 text-muted-foreground text-sm hover:bg-muted hover:text-foreground"
                      >
                        <Icon className="size-3.5 shrink-0" aria-hidden />
                        <span className="min-w-0 flex-1 truncate">
                          {connectionLabel(h, c, logins, shells)}
                        </span>
                      </button>
                    </ContextMenuTrigger>
                    <ContextMenuContent>
                      <ContextMenuItem onSelect={() => openConnection(h, c, 'horizontal')}>
                        Open in split
                      </ContextMenuItem>
                    </ContextMenuContent>
                  </ContextMenu>
                </li>
              )
            })}
          </ul>
        )}
      </li>
    )
  }

  return (
    <HostColumn>
      <nav aria-label="Hosts" className="flex w-full flex-col gap-2 overflow-hidden bg-sidebar p-2">
        <div className="flex items-center gap-1">
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search hosts"
            aria-label="Search hosts"
            className="h-8 text-sm"
          />
          <button type="button" aria-label="Hide the host list" onClick={toggle} className={strip}>
            <PanelLeftClose className="size-4" aria-hidden />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {pc && shown(pc) && <ul>{hostRow(pc)}</ul>}
          {groups.map((g) => {
            const list = g.hosts.filter(shown)
            if (list.length === 0) return null
            return (
              <section key={g.id} className="mt-2">
                {g.title && (
                  <h2 className="truncate px-1.5 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                    {g.title}
                  </h2>
                )}
                <ul>{list.map(hostRow)}</ul>
              </section>
            )
          })}
        </div>
      </nav>
    </HostColumn>
  )
}
