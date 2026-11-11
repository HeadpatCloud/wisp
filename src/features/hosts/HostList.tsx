import { ChevronDown, ChevronRight } from 'lucide-react'
import type { Host, SavedLogin, ShellInfo } from '@/bindings'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { ConnectionButton } from './ConnectionButton'
import { HostName } from './HostName'
import { ProfileIcon } from './ProfileIcon'

export interface HostGroup {
  folderId: string | null
  title: string
  hosts: Host[]
}

export function HostList({
  groups,
  folded,
  onFold,
  addressOf,
  logins,
  shells,
  onDragStart,
}: {
  groups: HostGroup[]
  folded: Set<string>
  onFold: (key: string) => void
  addressOf: (h: Host) => string
  logins: SavedLogin[]
  shells?: ShellInfo[]
  onDragStart: (hostId: string, e: React.PointerEvent) => void
}) {
  const select = useHostsUiStore((s) => s.select)
  return (
    <table className="w-full text-sm">
      <thead className="text-left text-muted-foreground text-xs">
        <tr>
          <th className="px-3 py-2 font-medium">Name</th>
          <th className="px-3 py-2 font-medium">Address</th>
          <th className="px-3 py-2 font-medium">Connections</th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => {
          const key = g.folderId ?? ''
          const open = !folded.has(key)
          return [
            g.title && (
              <tr key={`folder-${key}`} className="border-border border-t">
                <td colSpan={3} className="px-1 py-1.5">
                  <button
                    type="button"
                    aria-expanded={open}
                    onClick={() => onFold(key)}
                    data-drop-folder={key}
                    className="flex items-center gap-1 rounded px-2 py-0.5 font-semibold hover:bg-muted data-drop-over:bg-accent"
                  >
                    {open ? (
                      <ChevronDown className="size-3.5" aria-hidden />
                    ) : (
                      <ChevronRight className="size-3.5" aria-hidden />
                    )}
                    {g.title}
                  </button>
                </td>
              </tr>
            ),
            ...(open || !g.title
              ? g.hosts.map((h) => (
                  <tr
                    key={h.id}
                    onClick={() => select(h.id)}
                    data-drop-host={h.id}
                    onPointerDown={(e) => onDragStart(h.id, e)}
                    className="border-border border-t data-drop-over:bg-accent"
                  >
                    <td className="px-3 py-2 compact:py-1">
                      <span className="flex items-center gap-2 font-medium">
                        <ProfileIcon icon={h.icon} className="size-4 text-muted-foreground" />
                        <HostName host={h} />
                      </span>
                    </td>
                    <td className="px-3 py-2 text-muted-foreground">{addressOf(h)}</td>
                    <td className="px-3 py-2 compact:py-1">
                      <div className="flex flex-wrap gap-1.5">
                        {h.connections.map((c) => (
                          <ConnectionButton
                            key={c.id}
                            host={h}
                            connection={c}
                            logins={logins}
                            shells={shells}
                          />
                        ))}
                      </div>
                    </td>
                  </tr>
                ))
              : []),
          ]
        })}
      </tbody>
    </table>
  )
}
