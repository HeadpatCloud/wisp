import { LayoutGrid, List, Plug } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { Folder, Host } from '@/bindings'
import { connectOnce } from '@/features/connections/connectOnce'
import { confirmDeleteHost, deleteHostWithUndo } from '@/features/connections/deletes'
import { FirstStart } from '@/features/connections/FirstStart'
import { parseAddress } from '@/lib/address'
import {
  folderOrder,
  folderPath,
  hostMatches,
  THIS_COMPUTER,
  typeGroup,
  viaLine,
} from '@/lib/hosts'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { type TypeFilter, useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { HostCard } from './HostCard'
import { HostDetails } from './HostDetails'
import { type HostGroup, HostList } from './HostList'
import { hostDropMoves, neighbourFolder } from './moves'
import { openConnection } from './openConnection'
import { usePointerDrag } from './usePointerDrag'

const CHIPS: [TypeFilter, string][] = [
  ['all', 'All'],
  ['terminal', 'Terminal'],
  ['files', 'Files'],
  ['desktop', 'Remote desktop'],
]

// Folders in the order the nav shows them: each folder, then its subfolders.

function subtree(folderId: string, folders: Folder[]): Set<string> {
  const ids = new Set([folderId])
  let grew = true
  while (grew) {
    grew = false
    for (const f of folders) {
      if (f.parentId && ids.has(f.parentId) && !ids.has(f.id)) {
        ids.add(f.id)
        grew = true
      }
    }
  }
  return ids
}

export function HostsPage() {
  const hosts = useHostStore((s) => s.hosts)
  const folders = useHostStore((s) => s.folders)
  const tags = useHostStore((s) => s.tags)
  const logins = useHostStore((s) => s.logins)
  const folderId = useHostsUiStore((s) => s.folderId)
  const tagId = useHostsUiStore((s) => s.tagId)
  const typeFilter = useHostsUiStore((s) => s.typeFilter)
  const query = useHostsUiStore((s) => s.query)
  const setType = useHostsUiStore((s) => s.setType)
  const setQuery = useHostsUiStore((s) => s.setQuery)
  const selectedHostId = useHostsUiStore((s) => s.selectedHostId)
  const view = useSettingsStore((s) => s.settings.hostsView)
  const update = useSettingsStore((s) => s.update)
  const moveHosts = useHostStore((s) => s.moveHosts)
  const shells = useHostStore((s) => s.shells)
  const [folded, setFolded] = useState<Set<string>>(new Set())
  const drag = usePointerDrag<string>((hostId, target) => {
    const moves = hostDropMoves(hosts, hostId, target)
    if (moves.length) moveHosts(moves).catch(console.error)
  })

  const pc = hosts.find((h) => h.id === THIS_COMPUTER)
  const others = hosts.filter((h) => h.id !== THIS_COMPUTER)
  const inFolder = folderId ? subtree(folderId, folders) : null
  const visible = others.filter(
    (h) =>
      (!inFolder || (h.folderId !== null && inFolder.has(h.folderId))) &&
      (!tagId || (h.tagIds ?? []).includes(tagId)) &&
      (typeFilter === 'all' || h.connections.some((c) => typeGroup(c.kind.type) === typeFilter)) &&
      hostMatches(h, query, tags),
  )
  const byOrder = (list: Host[]) => [...list].sort((a, b) => a.order - b.order)
  const order = folderOrder(folders)
  const groups: HostGroup[] = order
    .map((id) => ({
      folderId: id,
      title: folderPath(id, folders),
      hosts: byOrder(visible.filter((h) => h.folderId === id)),
    }))
    .filter((g) => g.hosts.length > 0)
  const loose = byOrder(visible.filter((h) => !h.folderId || !order.includes(h.folderId)))
  if (loose.length > 0) groups.push({ folderId: null, title: '', hosts: loose })

  const showPc =
    pc &&
    !folderId &&
    !tagId &&
    (typeFilter === 'all' || typeFilter === 'terminal') &&
    hostMatches(pc, query, tags)
  const addressOf = (h: Host) => [h.address, viaLine(h, hosts)].filter(Boolean).join(' · ')
  const list = view === 'list'

  const selected = hosts.find((h) => h.id === selectedHostId)
  const shownGroups = list ? groups.filter((g) => !folded.has(g.folderId ?? '')) : groups
  const visual = [
    ...(showPc && pc ? [pc.id] : []),
    ...shownGroups.flatMap((g) => g.hosts.map((h) => h.id)),
  ]
  const dragged = hosts.find((h) => h.id === drag.dragging)
  const root = useRef<HTMLDivElement>(null)
  const current = useRef({ hosts, folders, visual, list })
  current.current = { hosts, folders, visual, list }

  // The page's own keys, only while the Hosts page is in front, focus is in it (or nowhere) and
  // nothing is being typed.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { hosts, folders, visual, list } = current.current
      if (useSessionStore.getState().activeTabId !== null) return
      const ui = useHostsUiStore.getState()
      if (ui.page !== 'hosts') return
      const t = e.target instanceof HTMLElement ? e.target : null
      if (t && t !== document.body && !root.current?.contains(t)) return
      if (t?.closest('input, textarea, select, [contenteditable="true"]')) return
      const sel = ui.selectedHostId
      const host = hosts.find((h) => h.id === sel)
      if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && e.altKey) {
        if (!host || host.id === THIS_COMPUTER) return
        const to = neighbourFolder(
          folderOrder(folders),
          host.folderId,
          e.key === 'ArrowDown' ? 1 : -1,
        )
        if (to === undefined) return
        e.preventDefault()
        moveHosts(hostDropMoves(hosts, host.id, { kind: 'folder', id: to })).catch(console.error)
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        if (visual.length === 0) return
        e.preventDefault()
        const at = sel ? visual.indexOf(sel) : -1
        const next =
          e.key === 'ArrowDown' ? Math.min(at + 1, visual.length - 1) : Math.max(at - 1, 0)
        ui.select(visual[next])
        // Focus follows, so Enter on the focused name opens the host that is selected.
        const names = root.current?.querySelectorAll<HTMLElement>('[data-host-name]') ?? []
        for (const b of names) if (b.dataset.hostName === visual[next]) b.focus()
      } else if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && list && host) {
        e.preventDefault()
        const key = host.folderId ?? ''
        setFolded((prev) => {
          const next = new Set(prev)
          if (e.key === 'ArrowLeft') next.add(key)
          else next.delete(key)
          return next
        })
      } else if (e.key === 'Enter' && host && !t?.closest('button')) {
        const main = host.connections[0]
        if (main) openConnection(host, main)
      } else if (e.key === 'F2' && host && host.id !== THIS_COMPUTER) {
        e.preventDefault()
        ui.setRenaming(host.id)
      } else if (e.key === 'Escape' && host) {
        ui.select(null)
      } else if (e.key === 'Delete' && host && host.id !== THIS_COMPUTER) {
        e.preventDefault()
        confirmDeleteHost(host)
          .then((ok) => (ok ? deleteHostWithUndo(host) : undefined))
          .catch(console.error)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [moveHosts])

  const parsed = parseAddress(query)
  const firstStart = others.length === 0 && !folderId && !tagId && typeFilter === 'all' && !query

  if (firstStart) {
    return (
      <div ref={root} className="flex h-full">
        <main className="min-w-0 flex-1 overflow-y-auto p-6">
          <FirstStart pc={pc ?? null} />
        </main>
      </div>
    )
  }

  return (
    <div ref={root} className="flex h-full">
      <main className="flex min-w-0 flex-1 flex-col gap-5 overflow-y-auto p-6">
        <header className="flex flex-wrap items-center gap-3">
          <h1 className="font-bold text-2xl tracking-tight">Hosts</h1>
          <label className="relative min-w-48 flex-1">
            <span className="sr-only">Search or connect</span>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search or connect"
              className="h-9 w-full rounded-md border border-input bg-background px-3 pr-16 text-sm"
            />
            <kbd className="-translate-y-1/2 absolute top-1/2 right-2 rounded border border-border px-1.5 text-muted-foreground text-xs">
              Ctrl K
            </kbd>
          </label>
          <fieldset className="flex gap-1">
            <legend className="sr-only">Filter by type</legend>
            {CHIPS.map(([value, label]) => (
              <button
                key={value}
                type="button"
                aria-pressed={typeFilter === value}
                onClick={() => setType(value)}
                className={cn(
                  'rounded-full border px-3 py-1 text-sm',
                  typeFilter === value
                    ? 'border-primary bg-primary text-primary-foreground'
                    : 'border-border hover:bg-muted',
                )}
              >
                {label}
              </button>
            ))}
          </fieldset>
          <fieldset className="flex rounded-md border border-border">
            <legend className="sr-only">View</legend>
            <button
              type="button"
              aria-label="Cards"
              aria-pressed={!list}
              onClick={() => update({ hostsView: 'cards' })}
              className={cn('rounded-l-md p-1.5', !list ? 'bg-muted' : 'hover:bg-muted')}
            >
              <LayoutGrid className="size-4" aria-hidden />
            </button>
            <button
              type="button"
              aria-label="List"
              aria-pressed={list}
              onClick={() => update({ hostsView: 'list' })}
              className={cn('rounded-r-md p-1.5', list ? 'bg-muted' : 'hover:bg-muted')}
            >
              <List className="size-4" aria-hidden />
            </button>
          </fieldset>
        </header>

        {parsed && (
          <button
            type="button"
            onClick={() => connectOnce(parsed)}
            className="flex items-center gap-2 self-start rounded-md border border-border px-3 py-1.5 text-sm hover:bg-muted"
          >
            <Plug className="size-4 text-muted-foreground" aria-hidden />
            Connect to {query.trim()} once
          </button>
        )}

        {list ? (
          <>
            <HostList
              groups={[
                ...(showPc && pc ? [{ folderId: THIS_COMPUTER, title: '', hosts: [pc] }] : []),
                ...groups,
              ]}
              folded={folded}
              onFold={(key) =>
                setFolded((prev) => {
                  const next = new Set(prev)
                  if (next.has(key)) next.delete(key)
                  else next.add(key)
                  return next
                })
              }
              addressOf={addressOf}
              logins={logins}
              shells={shells}
              onDragStart={drag.start}
            />
            <p className="flex flex-wrap gap-x-4 gap-y-1 text-muted-foreground text-xs">
              <span>↑ ↓ move</span>
              <span>← → fold folders</span>
              <span>Enter open first connection</span>
              <span>F2 rename</span>
              <span>Alt ↑ ↓ move between folders</span>
            </p>
          </>
        ) : (
          <>
            {showPc && pc && (
              <HostCard host={pc} address="" tags={tags} logins={logins} shells={shells} />
            )}
            {groups.map((g) => (
              <section key={g.folderId ?? ''} className="flex flex-col gap-2">
                {g.title && (
                  <h2
                    data-drop-folder={g.folderId ?? ''}
                    className="rounded font-semibold text-muted-foreground text-sm data-drop-over:bg-accent"
                  >
                    {g.title}
                  </h2>
                )}
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                  {g.hosts.map((h) => (
                    <HostCard
                      key={h.id}
                      host={h}
                      address={addressOf(h)}
                      tags={tags}
                      logins={logins}
                      onDragStart={drag.start}
                    />
                  ))}
                </div>
              </section>
            ))}
          </>
        )}

        {dragged && (
          <p role="status" className="text-muted-foreground text-sm">
            Drop to move {dragged.name} here
          </p>
        )}
        {others.length === 0 && (
          <div className="flex flex-col gap-1 text-sm">
            <p className="font-medium">No hosts yet.</p>
            <p className="text-muted-foreground">Hosts you add or import show up here.</p>
          </div>
        )}
      </main>
      {selected && <HostDetails host={selected} />}
    </div>
  )
}
