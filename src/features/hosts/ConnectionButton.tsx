import type { Connection, Host, SavedLogin, ShellInfo } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { connectionLabel, typeGroup } from '@/lib/hosts'
import { GROUP_ICON } from './icons'
import { openConnection } from './openConnection'

export function ConnectionButton({
  host,
  connection,
  logins,
  shells,
}: {
  host: Host
  connection: Connection
  logins: SavedLogin[]
  shells?: ShellInfo[]
}) {
  const Icon = GROUP_ICON[typeGroup(connection.kind.type)]
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            openConnection(host, connection)
          }}
          className="flex items-center gap-1.5 rounded-md border border-border bg-background px-2.5 py-1 text-sm hover:bg-muted"
        >
          <Icon className="size-3.5 text-muted-foreground" aria-hidden />
          {connectionLabel(host, connection, logins, shells)}
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => openConnection(host, connection, 'horizontal')}>
          Open in split
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
