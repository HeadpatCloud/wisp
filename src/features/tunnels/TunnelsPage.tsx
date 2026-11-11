import { useState } from 'react'
import type { Connection, Host, Tunnel } from '@/bindings'
import { Button } from '@/components/ui/button'
import { PageShell } from '@/components/ui/page-shell'
import {
  type TunnelDraft,
  tunnelDraft,
  tunnelErrors,
  tunnelFrom,
} from '@/features/connections/drafts'
import { SelectField } from '@/features/connections/fields'
import { TunnelRows } from '@/features/connections/TunnelRows'
import { formatBytes } from '@/lib/format'
import { connectionLabel, findConnection } from '@/lib/hosts'
import { forwardLabel, stopTunnel } from '@/lib/tunnels'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { useSessionStore } from '@/stores/sessionStore'
import { isRunning, type TunnelRuntime, useTunnelStore } from '@/stores/tunnelStore'
import { tabSession } from './run'
import { useTunnelStarter } from './useTunnelStarter'

interface Row {
  host: Host
  connection: Connection
  tunnel: Tunnel
}

function sshRows(hosts: Host[]): Row[] {
  return hosts.flatMap((host) =>
    host.connections.flatMap((connection) =>
      connection.kind.type === 'ssh'
        ? (connection.kind.tunnels ?? []).map((tunnel) => ({ host, connection, tunnel }))
        : [],
    ),
  )
}

function statusText(rt: TunnelRuntime | undefined): string {
  if (rt?.state === 'active') return 'Running'
  if (rt?.state === 'starting') return 'Starting…'
  if (rt?.state === 'error') return `Failed: ${rt.message ?? 'unknown error'}`
  return rt?.message ? `Stopped: ${rt.message}` : 'Stopped'
}

export function TunnelsPage({ onClose }: { onClose: () => void }) {
  const hosts = useHostStore((s) => s.hosts)
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  const saveHost = useHostStore((s) => s.saveHost)
  const byId = useTunnelStore((s) => s.byId)
  const background = useTunnelStore((s) => s.background)
  // Through which session a start would go depends on the open tabs.
  useSessionStore((s) => s.panes)
  const starter = useTunnelStarter()
  const [editing, setEditing] = useState<{ connectionId: string; drafts: TunnelDraft[] } | null>(
    null,
  )
  const [saveError, setSaveError] = useState<string | null>(null)
  const rows = sshRows(hosts)
  const sshConnections = hosts.flatMap((h) =>
    h.connections.filter((c) => c.kind.type === 'ssh').map((c) => ({ host: h, connection: c })),
  )

  const through = (row: Row, rt: TunnelRuntime | undefined) => {
    const k = row.connection.kind
    const jump = k.type === 'ssh' && k.jumpHostId ? findConnection(hosts, k.jumpHostId) : null
    const viaTab = isRunning(rt)
      ? !Object.values(background).includes(rt?.sessionId ?? '')
      : tabSession(row.connection.id) !== null
    return [
      `${row.host.name} · ${connectionLabel(row.host, row.connection, logins, shells)}`,
      viaTab ? `In the ${row.host.name} tab` : 'Background connection, no tab',
      jump ? `Via ${jump.host.name}` : '',
    ]
      .filter(Boolean)
      .join(' · ')
  }

  const beginNew = () => {
    const first = sshConnections[0]
    if (!first) return
    pick(first.connection.id)
  }
  const pick = (connectionId: string) => {
    const found = findConnection(hosts, connectionId)
    if (found?.connection.kind.type !== 'ssh') return
    setSaveError(null)
    setEditing({
      connectionId,
      drafts: [
        ...(found.connection.kind.tunnels ?? []).map(tunnelDraft),
        {
          id: crypto.randomUUID(),
          kind: 'local',
          bindHost: '127.0.0.1',
          bindPort: '8080',
          targetHost: '',
          targetPort: '',
          autoStart: false,
        },
      ],
    })
  }
  const errors = editing ? tunnelErrors(editing.drafts) : {}
  const save = () => {
    if (!editing) return
    const found = findConnection(hosts, editing.connectionId)
    if (found?.connection.kind.type !== 'ssh') return
    const tunnels = editing.drafts.map(tunnelFrom)
    const host: Host = {
      ...found.host,
      connections: found.host.connections.map((c) =>
        c.id === editing.connectionId && c.kind.type === 'ssh'
          ? { ...c, kind: { ...c.kind, tunnels } }
          : c,
      ),
    }
    saveHost(host)
      .then(() => setEditing(null))
      .catch((e: unknown) => setSaveError(e instanceof Error ? e.message : String(e)))
  }

  return (
    <PageShell
      title="Tunnels"
      footer={
        editing ? (
          <>
            <Button type="button" variant="ghost" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button type="button" disabled={Object.keys(errors).length > 0} onClick={save}>
              Save
            </Button>
          </>
        ) : (
          <>
            <Button type="button" variant="ghost" onClick={onClose}>
              Back
            </Button>
            <Button type="button" disabled={sshConnections.length === 0} onClick={beginNew}>
              New tunnel
            </Button>
          </>
        )
      }
    >
      {editing ? (
        <div className="grid max-w-2xl gap-4">
          <SelectField
            id="tunnel-connection"
            label="SSH connection"
            value={editing.connectionId}
            onChange={pick}
            options={sshConnections.map(({ host, connection }) => [
              connection.id,
              `${host.name} · ${connectionLabel(host, connection, logins, shells)}`,
            ])}
          />
          <TunnelRows
            tunnels={editing.drafts}
            onChange={(drafts) => setEditing({ ...editing, drafts })}
            errors={errors}
          />
          {saveError && (
            <p role="alert" className="text-destructive text-sm">
              {saveError}
            </p>
          )}
        </div>
      ) : rows.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          No tunnels yet. New tunnel forwards a port through one of your SSH connections.
        </p>
      ) : (
        <div className="flex flex-col gap-4">
          <ul className="flex flex-col gap-2">
            {rows.map((row) => {
              const rt = byId[row.tunnel.id]
              const running = isRunning(rt)
              return (
                <li
                  key={row.tunnel.id}
                  className="flex items-center gap-3 rounded-lg border border-border p-3 text-sm"
                >
                  <span
                    className={cn(
                      'size-2 shrink-0 rounded-full',
                      rt?.state === 'active' && 'bg-primary',
                      rt?.state === 'starting' && 'bg-muted-foreground',
                      rt?.state === 'error' && 'bg-destructive',
                      (!rt || rt.state === 'stopped') && 'bg-border',
                    )}
                  />
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium">{forwardLabel(row.tunnel)}</p>
                    <p className="truncate text-muted-foreground text-xs">{through(row, rt)}</p>
                    <p
                      className={cn(
                        'truncate text-xs',
                        rt?.state === 'error' ? 'text-destructive' : 'text-muted-foreground',
                      )}
                    >
                      {statusText(rt)}
                      {row.tunnel.autoStart && ' · Starts with its tab'}
                      {rt?.state === 'active' &&
                        ` · ↑${formatBytes(rt.bytesUp)} ↓${formatBytes(rt.bytesDown)}`}
                    </p>
                  </div>
                  {running ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => void stopTunnel(row.tunnel.id)}
                    >
                      Stop
                    </Button>
                  ) : (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => starter.start(row.connection.id, row.tunnel)}
                    >
                      {rt?.state === 'error' ? 'Retry' : 'Start'}
                    </Button>
                  )}
                </li>
              )
            })}
          </ul>
          <p className="text-muted-foreground text-xs">
            A tunnel runs in an open tab of its connection, or in a background connection that
            closes with its last tunnel.
          </p>
        </div>
      )}
      {starter.dialog}
    </PageShell>
  )
}
