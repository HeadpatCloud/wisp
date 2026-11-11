import { useEffect, useMemo, useRef, useState } from 'react'
import type { Tunnel } from '@/bindings'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { connectOnce, prefillOf } from '@/features/connections/connectOnce'
import { GROUP_ICON } from '@/features/hosts/icons'
import { openConnection } from '@/features/hosts/openConnection'
import { ProfileIcon } from '@/features/hosts/ProfileIcon'
import { tabSession } from '@/features/tunnels/run'
import { checkForUpdates } from '@/features/updater/updateNotice'
import { parseAddress } from '@/lib/address'
import {
  connectionLabel,
  effectiveAddress,
  folderPath,
  THIS_COMPUTER,
  typeGroup,
} from '@/lib/hosts'
import { forwardLabel, stopTunnel } from '@/lib/tunnels'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSessionStore } from '@/stores/sessionStore'
import { isRunning, useTunnelStore } from '@/stores/tunnelStore'

interface Item {
  id: string
  label: string
  hint?: string
  group: string
  icon?: React.ReactNode
  run: () => void
  // Terminals and file browsers can open beside the active tab's panes (Ctrl+Enter).
  split?: () => void
  edit?: { label: string; run: () => void }
}

// The second list Tab shows: what else the highlighted item can do.
function actionsOf(it: Item): Item[] {
  const out: Item[] = [{ ...it, id: `${it.id}:open`, label: 'Open', hint: undefined }]
  if (it.split)
    out.push({
      ...it,
      id: `${it.id}:split`,
      label: 'Open in split',
      hint: undefined,
      run: it.split,
    })
  if (it.edit)
    out.push({
      ...it,
      id: `${it.id}:edit`,
      label: it.edit.label,
      hint: undefined,
      run: it.edit.run,
    })
  return out
}

// Subsequence match: "wb1" finds "web-01". Returns null when it doesn't match.
function score(query: string, text: string): number | null {
  if (!query) return 0
  const t = text.toLowerCase()
  let i = 0
  let first = -1
  for (const ch of query) {
    const at = t.indexOf(ch, i)
    if (at === -1) return null
    if (first === -1) first = at
    i = at + 1
  }
  return first
}

export interface PaletteAction {
  id: string
  label: string
  run: () => void
}

export function CommandPalette({
  open,
  onOpenChange,
  actions = [],
  onStartTunnel,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  actions?: PaletteAction[]
  onStartTunnel?: (connectionId: string, tunnel: Tunnel) => void
}) {
  const hosts = useHostStore((s) => s.hosts)
  const folders = useHostStore((s) => s.folders)
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  const tabs = useSessionStore((s) => s.tabs)
  const runtimes = useTunnelStore((s) => s.byId)
  const panes = useSessionStore((s) => s.panes)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [actionsFor, setActionsFor] = useState<Item | null>(null)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (open) {
      setQuery('')
      setActive(0)
      setActionsFor(null)
    }
  }, [open])

  const items = useMemo<Item[]>(() => {
    const st = useSessionStore.getState()
    const ui = useHostsUiStore.getState()
    const out: Item[] = []
    for (const t of tabs) {
      const pane = t.kind === 'panes' ? panes[t.activePaneId] : undefined
      const Icon = GROUP_ICON[pane ? typeGroup(pane.kind) : 'desktop']
      out.push({
        id: `tab:${t.id}`,
        label: t.kind === 'panes' ? (pane?.title ?? '') : t.title,
        hint: 'Switch to tab',
        group: 'Open tabs',
        icon: <Icon className="size-4 text-muted-foreground" aria-hidden />,
        run: () => st.setActiveTab(t.id),
      })
    }
    for (const host of hosts) {
      const folder = host.folderId ? folderPath(host.folderId, folders) : ''
      for (const c of host.connections) {
        const k = c.kind
        const notes = [
          k.type === 'local' ? '' : effectiveAddress(host, c),
          folder,
          k.type === 'sftp' && !k.own ? 'same login as SSH' : '',
        ]
        const desktop = typeGroup(k.type) === 'desktop'
        for (const t of k.type === 'ssh' ? (k.tunnels ?? []) : []) {
          const running = isRunning(runtimes[t.id])
          out.push({
            id: `tunnel:${t.id}`,
            label: `${running ? 'Stop' : 'Start'} tunnel ${forwardLabel(t)}`,
            hint: running
              ? undefined
              : tabSession(c.id)
                ? `In the ${host.name} tab`
                : `Opens ${host.name} · ${connectionLabel(host, c, logins, shells)} in the background`,
            group: 'Actions',
            run: () => (running ? void stopTunnel(t.id) : onStartTunnel?.(c.id, t)),
          })
        }
        out.push({
          id: `conn:${c.id}`,
          label: `${host.name} · ${connectionLabel(host, c, logins, shells)}`,
          hint: notes.filter(Boolean).join(' · ') || undefined,
          group: 'Connect',
          icon: <ProfileIcon icon={host.icon} className="size-4 text-muted-foreground" />,
          run: () => openConnection(host, c),
          split: desktop ? undefined : () => openConnection(host, c, 'horizontal'),
          edit:
            host.id === THIS_COMPUTER
              ? undefined
              : { label: `Edit ${host.name}`, run: () => ui.openEditor(host.id) },
        })
      }
    }
    out.push(
      {
        id: 'act:local',
        label: 'New local shell',
        group: 'Actions',
        run: () => st.openLocalShell(),
      },
      {
        id: 'act:new',
        label: 'New connection',
        group: 'Actions',
        run: () => ui.openNewConnection(),
      },
      {
        id: 'act:hosts',
        label: 'Hosts',
        group: 'Actions',
        run: () => ui.openPage('hosts'),
      },
      { id: 'act:tunnels', label: 'Tunnels', group: 'Actions', run: () => ui.openPage('tunnels') },
      {
        id: 'act:keys',
        label: 'Keys & passwords',
        group: 'Actions',
        run: () => ui.openPage('keys'),
      },
      {
        id: 'act:knownHosts',
        label: 'Known hosts',
        group: 'Actions',
        run: () => ui.openPage('knownHosts'),
      },
      {
        id: 'act:settings',
        label: 'Settings',
        group: 'Actions',
        run: () => ui.openPage('settings'),
      },
      {
        id: 'act:updates',
        label: 'Check for updates',
        group: 'Actions',
        run: () => void checkForUpdates(),
      },
      ...actions.map((a) => ({ ...a, id: `act:${a.id}`, group: 'Actions' })),
      ...hosts
        .filter((h) => h.id !== THIS_COMPUTER)
        .map((h) => ({
          id: `edit:${h.id}`,
          label: `Edit ${h.name}`,
          group: 'Actions',
          run: () => ui.openEditor(h.id),
        })),
    )
    return out
  }, [hosts, folders, logins, shells, tabs, panes, runtimes, actions, onStartTunnel])

  const typed = query.trim()
  const q = typed.toLowerCase()
  const results = useMemo(() => {
    const scored = items
      .map((it) => ({ it, s: score(q, `${it.label} ${it.hint ?? ''}`) }))
      .filter((x): x is { it: Item; s: number } => x.s !== null)
    scored.sort((a, b) => a.s - b.s)
    // Saved matches come first, so Enter still opens them; then what the typed text itself offers.
    const parsed = parseAddress(typed)
    const fromText: Item[] = []
    if (parsed) {
      fromText.push({
        id: 'once',
        label: `Connect to ${typed} once`,
        group: 'Connect',
        run: () => connectOnce(parsed),
      })
    }
    if (typed) {
      fromText.push({
        id: 'new',
        label: `New connection to "${typed}"…`,
        group: 'Actions',
        run: () => useHostsUiStore.getState().openNewConnection(parsed ? prefillOf(parsed) : {}),
      })
    }
    return [...scored.slice(0, 50).map((x) => x.it), ...fromText]
  }, [items, q, typed])

  const rows = actionsFor ? actionsOf(actionsFor) : results
  const clamped = Math.min(active, Math.max(0, rows.length - 1))

  // biome-ignore lint/correctness/useExhaustiveDependencies: follows the highlighted row
  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [clamped])

  const run = (go: () => void) => {
    onOpenChange(false)
    go()
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="top-24 translate-y-0 gap-2 p-2"
        onEscapeKeyDown={(e) => {
          if (!actionsFor) return
          e.preventDefault()
          setActionsFor(null)
          setActive(0)
        }}
      >
        <DialogHeader className="sr-only">
          <DialogTitle>Command palette</DialogTitle>
        </DialogHeader>
        <Input
          autoFocus
          value={query}
          placeholder="Search hosts and actions…"
          onChange={(e) => {
            setQuery(e.target.value)
            setActive(0)
            setActionsFor(null)
          }}
          onKeyDown={(e) => {
            const row = rows[clamped]
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              setActive((a) => Math.min(a + 1, rows.length - 1))
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              setActive((a) => Math.max(a - 1, 0))
            } else if (e.key === 'Enter' && row) {
              e.preventDefault()
              const split = !actionsFor && (e.ctrlKey || e.metaKey) ? row.split : undefined
              run(split ?? row.run)
            } else if (e.key === 'Tab') {
              // Focus leaving the box would leave the arrows and Enter acting on nothing.
              e.preventDefault()
              if (e.shiftKey || actionsFor || !(row?.split || row?.edit)) return
              setActionsFor(row)
              setActive(0)
            }
          }}
        />
        <div ref={listRef} className="max-h-80 overflow-y-auto">
          {rows.length === 0 ? (
            <p className="px-2 py-6 text-center text-muted-foreground text-sm">No matches.</p>
          ) : (
            rows.map((it, i) => (
              <button
                key={it.id}
                type="button"
                data-active={i === clamped}
                onMouseEnter={() => setActive(i)}
                onClick={() => run(it.run)}
                className={cn(
                  'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm',
                  i === clamped && 'bg-muted',
                )}
              >
                {it.icon}
                <span className="min-w-0 flex-1 truncate">{it.label}</span>
                {it.hint && (
                  <span className="shrink-0 truncate text-muted-foreground text-xs">{it.hint}</span>
                )}
                {!actionsFor && (
                  <span className="shrink-0 text-muted-foreground text-xs">{it.group}</span>
                )}
              </button>
            ))
          )}
        </div>
        <p className="flex gap-3 border-border border-t px-2 pt-2 text-muted-foreground text-xs">
          <span>↵ open</span>
          <span>Ctrl+↵ open in split</span>
          <span>Tab actions</span>
          <span>Esc {actionsFor ? 'back' : 'close'}</span>
        </p>
      </DialogContent>
    </Dialog>
  )
}
