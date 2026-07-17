import { File, Folder } from 'lucide-react'
import type { SftpEntry } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 ** 2).toFixed(1)} MB`
}

export function FileList({
  entries,
  selected,
  onRowClick,
  onEnter,
  onDownload,
  onRename,
  onDelete,
}: {
  entries: SftpEntry[]
  selected: ReadonlySet<string>
  onRowClick: (e: SftpEntry, mods: { toggle: boolean; range: boolean }) => void
  onEnter: (e: SftpEntry) => void
  onDownload: (e: SftpEntry) => void
  onRename: (e: SftpEntry) => void
  onDelete: (e: SftpEntry) => void
}) {
  return (
    <ScrollArea className="min-h-0 flex-1">
      <ul className="text-sm">
        {entries.map((e) => (
          <ContextMenu key={e.path}>
            <ContextMenuTrigger asChild>
              {/* biome-ignore lint/a11y/useKeyWithClickEvents: selection has browser-level keys (Ctrl+A/Escape/Delete) */}
              <li
                data-selected={selected.has(e.path) || undefined}
                onClick={(ev) =>
                  onRowClick(e, { toggle: ev.ctrlKey || ev.metaKey, range: ev.shiftKey })
                }
                onDoubleClick={() => (e.isDir ? onEnter(e) : onDownload(e))}
                className={cn(
                  'flex cursor-default select-none items-center gap-2 px-2 py-1',
                  selected.has(e.path) ? 'bg-muted' : 'hover:bg-muted/50',
                )}
              >
                {e.isDir ? (
                  <Folder className="size-4 shrink-0 text-muted-foreground" />
                ) : (
                  <File className="size-4 shrink-0 text-muted-foreground" />
                )}
                <span className="min-w-0 flex-1 truncate">{e.name}</span>
                <span className="w-20 shrink-0 text-right text-muted-foreground text-xs tabular-nums">
                  {e.isDir ? '' : fmtSize(e.size)}
                </span>
                <span className="w-24 shrink-0 text-right text-muted-foreground text-xs">
                  {e.modified ? new Date(e.modified * 1000).toLocaleDateString() : ''}
                </span>
              </li>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem onSelect={() => onDownload(e)}>Download</ContextMenuItem>
              <ContextMenuItem onSelect={() => onRename(e)}>Rename</ContextMenuItem>
              <ContextMenuItem onSelect={() => onDelete(e)}>Delete</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        ))}
      </ul>
    </ScrollArea>
  )
}
