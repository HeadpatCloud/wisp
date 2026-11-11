import { GripVertical, Plus, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { type Connection, commands, type Host, type SavedLogin } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { useTunnelStarter } from '@/features/tunnels/useTunnelStarter'
import { connectionLabel, effectiveAddress, folderPath, typeGroup } from '@/lib/hosts'
import { unwrap } from '@/lib/ipc'
import { forwardLabel, stopTunnel } from '@/lib/tunnels'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { isRunning, useTunnelStore } from '@/stores/tunnelStore'
import { openConnection, openFiles } from './openConnection'
import { usePointerDrag } from './usePointerDrag'

export function reorderConnections(host: Host, from: number, to: number): Host {
  const connections = [...host.connections]
  const [moved] = connections.splice(from, 1)
  connections.splice(to, 0, moved)
  return { ...host, connections }
}

function note(c: Connection, logins: SavedLogin[]): string {
  const k = c.kind
  if (k.type === 'sftp') {
    if (!k.own) return 'Same login as SSH'
    return k.own.login.kind === 'own' ? k.own.login.username : ''
  }
  if (k.type !== 'ssh') return ''
  if (k.login.kind === 'own') return k.login.username
  const { loginId } = k.login
  const saved = logins.find((l) => l.id === loginId)
  return saved ? `Login "${saved.name}" · ${saved.authMethod}` : ''
}

export function HostDetails({ host }: { host: Host }) {
  const folders = useHostStore((s) => s.folders)
  const tags = useHostStore((s) => s.tags)
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  const saveHost = useHostStore((s) => s.saveHost)
  const select = useHostsUiStore((s) => s.select)
  const openNewConnection = useHostsUiStore((s) => s.openNewConnection)
  const openEditor = useHostsUiStore((s) => s.openEditor)
  const runtimes = useTunnelStore((s) => s.byId)
  const mainSsh = host.connections.find((c) => c.kind.type === 'ssh')
  const sshKey =
    mainSsh?.kind.type === 'ssh' ? `${effectiveAddress(host, mainSsh)}:${mainSsh.kind.port}` : null
  const [trusted, setTrusted] = useState<boolean | null>(null)
  // Read when the panel opens; a key trusted meanwhile shows the next time.
  useEffect(() => {
    if (!sshKey) return
    commands
      .knownHostsList()
      .then((r) => setTrusted(unwrap(r).some((e) => e.key === sshKey)))
      .catch(console.error)
  }, [sshKey])
  const starter = useTunnelStarter()
  const tunnels = host.connections.flatMap((c) =>
    c.kind.type === 'ssh' ? (c.kind.tunnels ?? []).map((t) => ({ c, t })) : [],
  )

  const move = (from: number, to: number) => {
    if (to < 0 || to >= host.connections.length || to === from) return
    saveHost(reorderConnections(host, from, to)).catch(console.error)
  }
  const drag = usePointerDrag<number>((from, target) => {
    if (target.kind === 'conn') move(from, target.index)
  })
  const where = [folderPath(host.folderId, folders), host.address].filter(Boolean).join(' · ')
  const own = tags.filter((t) => (host.tagIds ?? []).includes(t.id))

  return (
    <aside
      aria-label="Selected host"
      className="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto border-border border-l bg-card p-4"
    >
      <header className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-semibold text-lg">{host.name}</h2>
          {where && <p className="truncate text-muted-foreground text-sm">{where}</p>}
          <button
            type="button"
            onClick={() => openEditor(host.id)}
            className="mt-2 rounded-md border border-border px-2.5 py-1 text-sm hover:bg-muted"
          >
            Edit host
          </button>
          {own.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {own.map((t) => (
                <span key={t.id} className="rounded-full bg-muted px-2 py-0.5 font-medium text-xs">
                  {t.name}
                </span>
              ))}
            </div>
          )}
        </div>
        <button
          type="button"
          aria-label="Close details"
          onClick={() => select(null)}
          className="rounded p-1 hover:bg-muted"
        >
          <X className="size-4" aria-hidden />
        </button>
      </header>
      <section className="flex flex-col gap-2">
        <div className="flex items-baseline justify-between">
          <h3 className="font-semibold text-sm">Connections</h3>
          <span className="text-muted-foreground text-xs">Enter opens the first</span>
        </div>
        <ul className="flex flex-col gap-1.5">
          {host.connections.map((c, i) => {
            const label = connectionLabel(host, c, logins, shells)
            const files = typeGroup(c.kind.type) === 'files'
            const sub = note(c, logins)
            return (
              <ContextMenu key={c.id}>
                <ContextMenuTrigger asChild>
                  <li
                    data-drop-conn={i}
                    className="flex items-center gap-2 rounded-lg border border-border bg-background p-2 data-drop-over:ring-2 data-drop-over:ring-ring"
                  >
                    <button
                      type="button"
                      aria-label={`Drag to reorder ${label}`}
                      onPointerDown={(e) => drag.start(i, e)}
                      onKeyDown={(e) => {
                        if (!e.altKey) return
                        // The Hosts page also uses Alt+arrows, to move the host between folders.
                        e.stopPropagation()
                        if (e.key === 'ArrowUp') move(i, i - 1)
                        else if (e.key === 'ArrowDown') move(i, i + 1)
                        else return
                        e.preventDefault()
                      }}
                      className="cursor-grab rounded p-0.5 text-muted-foreground hover:bg-muted"
                    >
                      <GripVertical className="size-4" aria-hidden />
                    </button>
                    <div className="min-w-0 flex-1">
                      <p className="flex items-center gap-1.5 truncate font-medium text-sm">
                        {label}
                        {i === 0 && (
                          <span className="rounded bg-muted px-1.5 font-medium text-muted-foreground text-xs">
                            Main
                          </span>
                        )}
                      </p>
                      {sub && <p className="truncate text-muted-foreground text-xs">{sub}</p>}
                    </div>
                    {c.kind.type === 'ssh' && (
                      <button
                        type="button"
                        onClick={() => openFiles(host, c.id)}
                        className="rounded-md px-2 py-1 text-sm hover:bg-muted"
                      >
                        Open files
                      </button>
                    )}
                    <button
                      type="button"
                      aria-label={`${files ? 'Open' : 'Connect'} ${label}`}
                      onClick={() => openConnection(host, c)}
                      className="rounded-md border border-border px-2.5 py-1 text-sm hover:bg-muted"
                    >
                      {files ? 'Open' : 'Connect'}
                    </button>
                  </li>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  <ContextMenuItem onSelect={() => openConnection(host, c, 'horizontal')}>
                    Open in split
                  </ContextMenuItem>
                  {c.kind.type === 'ssh' && (
                    <ContextMenuItem onSelect={() => openFiles(host, c.id, 'horizontal')}>
                      Open files in split
                    </ContextMenuItem>
                  )}
                </ContextMenuContent>
              </ContextMenu>
            )
          })}
        </ul>
        {sshKey && trusted !== null && (
          <p className="text-muted-foreground text-xs">
            {trusted ? 'Host key trusted' : 'Host key not trusted yet'}
          </p>
        )}
        <button
          type="button"
          onClick={() => openNewConnection({ hostId: host.id })}
          className="flex items-center gap-1.5 self-start rounded-md px-2 py-1 text-sm hover:bg-muted"
        >
          <Plus className="size-4" aria-hidden />
          Add a connection
        </button>
      </section>
      {tunnels.length > 0 && (
        <section className="flex flex-col gap-2">
          <h3 className="font-semibold text-sm">Tunnels</h3>
          <ul className="flex flex-col gap-1.5">
            {tunnels.map(({ c, t }) => {
              const rt = runtimes[t.id]
              const on = isRunning(rt)
              return (
                <li
                  key={t.id}
                  className="flex items-center gap-2 rounded-lg border border-border bg-background p-2 text-sm"
                >
                  <span className="min-w-0 flex-1 truncate">{forwardLabel(t)}</span>
                  <span className="text-muted-foreground text-xs">
                    {on ? 'Running' : rt?.state === 'error' ? 'Failed' : 'Stopped'}
                  </span>
                  <button
                    type="button"
                    onClick={() => (on ? void stopTunnel(t.id) : starter.start(c.id, t))}
                    className="rounded-md border border-border px-2.5 py-1 text-sm hover:bg-muted"
                  >
                    {on ? 'Stop' : 'Start'}
                  </button>
                </li>
              )
            })}
          </ul>
        </section>
      )}
      {starter.dialog}
    </aside>
  )
}
