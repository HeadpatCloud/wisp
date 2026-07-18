import {
  ChevronDown,
  ChevronUp,
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileJson,
  FileKey,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Folder,
  FolderSymlink,
  type LucideIcon,
} from 'lucide-react'
import { useEffect, useRef } from 'react'
import type { SftpEntry } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { formatBytes } from '@/lib/format'
import { cn } from '@/lib/utils'

export type SortKey = 'name' | 'size' | 'modified'

const BY_EXT: Record<string, LucideIcon> = Object.fromEntries([
  ...['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif', 'tiff'].map((e) => [
    e,
    FileImage,
  ]),
  ...['mp4', 'mkv', 'mov', 'avi', 'webm', 'flv', 'wmv'].map((e) => [e, FileVideo]),
  ...['mp3', 'wav', 'flac', 'ogg', 'm4a', 'aac', 'opus'].map((e) => [e, FileAudio]),
  ...['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar', 'zst', 'jar'].map((e) => [
    e,
    FileArchive,
  ]),
  ...[
    'js',
    'mjs',
    'cjs',
    'ts',
    'tsx',
    'jsx',
    'rs',
    'go',
    'py',
    'rb',
    'java',
    'c',
    'h',
    'cpp',
    'hpp',
    'cs',
    'php',
    'swift',
    'kt',
    'lua',
    'sh',
    'bash',
    'zsh',
    'fish',
    'ps1',
    'sql',
    'html',
    'css',
    'scss',
  ].map((e) => [e, FileCode]),
  ...['json', 'yaml', 'yml', 'toml', 'xml', 'ini', 'conf', 'cfg', 'env'].map((e) => [e, FileJson]),
  ...['csv', 'tsv', 'xlsx', 'xls', 'ods'].map((e) => [e, FileSpreadsheet]),
  ...['pem', 'key', 'crt', 'cer', 'pub', 'gpg', 'asc'].map((e) => [e, FileKey]),
  ...['md', 'txt', 'log', 'rst', 'pdf', 'doc', 'docx', 'rtf', 'lock'].map((e) => [e, FileText]),
])

function iconFor(e: SftpEntry): LucideIcon {
  if (e.isDir) return e.isSymlink ? FolderSymlink : Folder
  return BY_EXT[e.name.split('.').pop()?.toLowerCase() ?? ''] ?? File
}

function Column({
  label,
  sortKey,
  sort,
  onSort,
  className,
}: {
  label: string
  sortKey: SortKey
  sort: { key: SortKey; desc: boolean }
  onSort: (k: SortKey) => void
  className?: string
}) {
  return (
    <button
      type="button"
      onClick={() => onSort(sortKey)}
      className={cn('flex items-center gap-1 hover:text-foreground', className)}
    >
      {label}
      {sort.key === sortKey &&
        (sort.desc ? <ChevronDown className="size-3" /> : <ChevronUp className="size-3" />)}
    </button>
  )
}

export function FileList({
  entries,
  selected,
  sort,
  onSort,
  onRowClick,
  onEnter,
  onDownload,
  onRename,
  onEdit,
  onDelete,
  onCopyPath,
  onCopyName,
  onCopyUrl,
  onSignedLink,
  renaming,
  onRenameSubmit,
  onRenameCancel,
  keyNav,
}: {
  entries: SftpEntry[]
  selected: ReadonlySet<string>
  sort: { key: SortKey; desc: boolean }
  onSort: (k: SortKey) => void
  onRowClick: (e: SftpEntry, mods: { toggle: boolean; range: boolean }) => void
  onEnter: (e: SftpEntry) => void
  onDownload: (e: SftpEntry) => void
  onRename: (e: SftpEntry) => void
  onEdit: (e: SftpEntry) => void
  onDelete: (e: SftpEntry) => void
  onCopyPath: (e: SftpEntry) => void
  onCopyName: (e: SftpEntry) => void
  onCopyUrl?: (e: SftpEntry) => void
  onSignedLink?: (e: SftpEntry) => void
  renaming: string | null
  onRenameSubmit: (e: SftpEntry, name: string) => void
  onRenameCancel: () => void
  keyNav: { current: boolean }
}) {
  const listRef = useRef<HTMLUListElement>(null)

  // Keep the keyboard cursor visible as arrows move the single selection. Scrolling on a
  // plain click would yank the list out from under the pointer (and the context menu).
  useEffect(() => {
    if (selected.size !== 1 || !keyNav.current) return
    listRef.current?.querySelector('[data-selected]')?.scrollIntoView({ block: 'nearest' })
  }, [selected, keyNav])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-border border-b px-2 py-1 text-muted-foreground text-xs">
        <span className="size-4 shrink-0" />
        <Column
          label="Name"
          sortKey="name"
          sort={sort}
          onSort={onSort}
          className="min-w-0 flex-1"
        />
        <Column
          label="Size"
          sortKey="size"
          sort={sort}
          onSort={onSort}
          className="w-20 shrink-0 justify-end"
        />
        <Column
          label="Modified"
          sortKey="modified"
          sort={sort}
          onSort={onSort}
          className="w-24 shrink-0 justify-end"
        />
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <ul ref={listRef} className="text-sm">
          {entries.map((e) => {
            const Icon = iconFor(e)
            const isSelected = selected.has(e.path)
            const count = isSelected ? selected.size : 1
            return (
              <ContextMenu key={e.path}>
                <ContextMenuTrigger asChild>
                  {/* biome-ignore lint/a11y/useKeyWithClickEvents: selection uses browser-level keys (Ctrl+A/Escape/Delete) */}
                  <li
                    data-selected={isSelected || undefined}
                    onClick={(ev) =>
                      onRowClick(e, { toggle: ev.ctrlKey || ev.metaKey, range: ev.shiftKey })
                    }
                    onContextMenu={() => {
                      if (!isSelected) onRowClick(e, { toggle: false, range: false })
                    }}
                    onDoubleClick={() => (e.isDir ? onEnter(e) : onDownload(e))}
                    className={cn(
                      'flex cursor-default select-none items-center gap-2 px-2 py-1 data-[state=open]:bg-muted',
                      isSelected ? 'bg-muted' : 'hover:bg-muted/50',
                    )}
                  >
                    <Icon
                      className={cn(
                        'size-4 shrink-0',
                        e.isDir ? 'text-primary' : 'text-muted-foreground',
                      )}
                    />
                    {renaming === e.path ? (
                      <Input
                        autoFocus
                        defaultValue={e.name}
                        onClick={(ev) => ev.stopPropagation()}
                        onDoubleClick={(ev) => ev.stopPropagation()}
                        onFocus={(ev) => ev.currentTarget.select()}
                        onBlur={onRenameCancel}
                        onKeyDown={(ev) => {
                          ev.stopPropagation()
                          if (ev.key === 'Enter') {
                            const v = ev.currentTarget.value.trim()
                            if (v && v !== e.name) onRenameSubmit(e, v)
                            else onRenameCancel()
                          } else if (ev.key === 'Escape') {
                            onRenameCancel()
                          }
                        }}
                        className="h-6 min-w-0 flex-1 px-1 text-sm"
                      />
                    ) : (
                      <span className={cn('min-w-0 flex-1 truncate', e.isSymlink && 'italic')}>
                        {e.name}
                      </span>
                    )}
                    <span className="w-20 shrink-0 text-right text-muted-foreground text-xs tabular-nums">
                      {e.isDir ? '' : formatBytes(e.size)}
                    </span>
                    <span className="w-24 shrink-0 text-right text-muted-foreground text-xs">
                      {e.modified ? new Date(e.modified * 1000).toLocaleDateString() : ''}
                    </span>
                  </li>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  <ContextMenuLabel className="max-w-56 truncate font-normal text-muted-foreground text-xs">
                    {count > 1 ? `${count} items` : e.name}
                  </ContextMenuLabel>
                  <ContextMenuSeparator />
                  {e.isDir && count === 1 && (
                    <ContextMenuItem onSelect={() => onEnter(e)}>Open</ContextMenuItem>
                  )}
                  {!e.isDir && count === 1 && (
                    <ContextMenuItem onSelect={() => onEdit(e)}>Edit locally…</ContextMenuItem>
                  )}
                  <ContextMenuItem onSelect={() => onDownload(e)}>Download</ContextMenuItem>
                  {count === 1 && (
                    <ContextMenuItem onSelect={() => onCopyName(e)}>Copy name</ContextMenuItem>
                  )}
                  <ContextMenuItem onSelect={() => onCopyPath(e)}>
                    {count > 1 ? 'Copy paths' : 'Copy path'}
                  </ContextMenuItem>
                  {onCopyUrl && count === 1 && (
                    <ContextMenuItem onSelect={() => onCopyUrl(e)}>Copy URL</ContextMenuItem>
                  )}
                  {onSignedLink && count === 1 && !e.isDir && (
                    <ContextMenuItem onSelect={() => onSignedLink(e)}>
                      Shareable link…
                    </ContextMenuItem>
                  )}
                  {count === 1 && (
                    <ContextMenuItem onSelect={() => onRename(e)}>Rename</ContextMenuItem>
                  )}
                  <ContextMenuSeparator />
                  <ContextMenuItem variant="destructive" onSelect={() => onDelete(e)}>
                    Delete
                  </ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            )
          })}
        </ul>
      </ScrollArea>
    </div>
  )
}
