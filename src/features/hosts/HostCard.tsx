import type { Host, SavedLogin, ShellInfo, Tag } from '@/bindings'
import { tagColor } from '@/lib/tagColors'
import { cn } from '@/lib/utils'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { ConnectionButton } from './ConnectionButton'
import { HostName } from './HostName'
import { ProfileIcon } from './ProfileIcon'

export function HostCard({
  host,
  address,
  tags,
  logins,
  shells,
  onDragStart,
}: {
  host: Host
  address: string
  tags: Tag[]
  logins: SavedLogin[]
  shells?: ShellInfo[]
  onDragStart?: (hostId: string, e: React.PointerEvent) => void
}) {
  const own = tags.filter((t) => (host.tagIds ?? []).includes(t.id))
  const selected = useHostsUiStore((s) => s.selectedHostId === host.id)
  const select = useHostsUiStore((s) => s.select)
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: the host name button is the keyboard way to select
    <article
      onClick={() => select(host.id)}
      data-drop-host={onDragStart ? host.id : undefined}
      onPointerDown={onDragStart && ((e) => onDragStart(host.id, e))}
      className={cn(
        'flex flex-col gap-3 rounded-xl border bg-card p-4 compact:gap-2 compact:p-2.5',
        selected ? 'border-primary ring-2 ring-ring' : 'border-border',
        'data-drop-over:ring-2 data-drop-over:ring-ring',
      )}
    >
      <div className="flex items-start gap-3">
        <span
          className={cn(
            'flex size-9 shrink-0 items-center justify-center rounded-lg',
            tagColor(host.color)?.dot ?? 'bg-muted',
          )}
        >
          <ProfileIcon
            icon={host.icon}
            className={cn(
              'size-5',
              host.color ? 'text-primary-foreground' : 'text-muted-foreground',
            )}
          />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate font-semibold">
            <HostName host={host} />
          </h3>
          {address && <p className="truncate text-muted-foreground text-sm">{address}</p>}
        </div>
        {own.map((t) => (
          <span
            key={t.id}
            className="flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 font-medium text-xs"
          >
            {t.showColor && (
              <span className={cn('size-1.5 rounded-full', tagColor(t.color)?.dot)} />
            )}
            {t.name}
          </span>
        ))}
      </div>
      {host.connections.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {host.connections.map((c) => (
            <ConnectionButton
              key={c.id}
              host={host}
              connection={c}
              logins={logins}
              shells={shells}
            />
          ))}
        </div>
      )}
    </article>
  )
}
