import {
  Cable,
  Folder as FolderIcon,
  FolderPlus,
  KeyRound,
  LayoutGrid,
  Plus,
  Settings,
  ShieldCheck,
} from 'lucide-react'
import { useState } from 'react'
import type { Folder, Host } from '@/bindings'
import { Button } from '@/components/ui/button'
import { THIS_COMPUTER } from '@/lib/hosts'
import { tagColor } from '@/lib/tagColors'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { isRunning, useTunnelStore } from '@/stores/tunnelStore'
import { InlineName } from './InlineName'
import { folderDropMove } from './moves'
import { usePointerDrag } from './usePointerDrag'

function countUnder(folderId: string, folders: Folder[], hosts: Host[]): number {
  return (
    hosts.filter((h) => h.folderId === folderId).length +
    folders
      .filter((f) => f.parentId === folderId)
      .reduce((n, f) => n + countUnder(f.id, folders, hosts), 0)
  )
}

const row =
  'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm compact:py-1 data-drop-over:bg-accent data-drop-over:ring-2 data-drop-over:ring-ring'

export function HostsNav() {
  const folders = useHostStore((s) => s.folders)
  const allHosts = useHostStore((s) => s.hosts)
  const tags = useHostStore((s) => s.tags)
  const page = useHostsUiStore((s) => s.page)
  const folderId = useHostsUiStore((s) => s.folderId)
  const tagId = useHostsUiStore((s) => s.tagId)
  const openPage = useHostsUiStore((s) => s.openPage)
  const openNewConnection = useHostsUiStore((s) => s.openNewConnection)
  const setFolder = useHostsUiStore((s) => s.setFolder)
  const setTag = useHostsUiStore((s) => s.setTag)
  const tunnelsOn = useTunnelStore((s) => Object.values(s.byId).filter(isRunning).length)
  const saveFolder = useHostStore((s) => s.saveFolder)
  const moveFolders = useHostStore((s) => s.moveFolders)
  const [creating, setCreating] = useState(false)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const drag = usePointerDrag<string>((id, target) => {
    const move = folderDropMove(folders, id, target)
    if (!move) return
    setError(null)
    moveFolders([move]).catch((e) => setError(e instanceof Error ? e.message : String(e)))
  })
  const fail = (e: unknown) => setError(e instanceof Error ? e.message : String(e))
  const hosts = allHosts.filter((h) => h.id !== THIS_COMPUTER)
  const known = new Set(folders.map((f) => f.id))
  const byOrder = (list: Folder[]) => [...list].sort((a, b) => a.order - b.order)
  const onHosts = page === 'hosts'

  const folderRows = (parentId: string | null) => {
    const children = byOrder(
      folders.filter((f) =>
        parentId === null ? !f.parentId || !known.has(f.parentId) : f.parentId === parentId,
      ),
    )
    if (children.length === 0) return null
    return (
      <ul className={cn(parentId !== null && 'pl-3')}>
        {children.map((f) => (
          <li key={f.id}>
            {renaming === f.id ? (
              <InlineName
                initial={f.name}
                label="Folder name"
                onSave={(name) => {
                  setRenaming(null)
                  saveFolder({ ...f, name }).catch(fail)
                }}
                onCancel={() => setRenaming(null)}
              />
            ) : (
              <button
                type="button"
                data-drop-folder={f.id}
                onPointerDown={(e) => drag.start(f.id, e)}
                onKeyDown={(e) => {
                  if (e.key !== 'F2') return
                  e.preventDefault()
                  setRenaming(f.id)
                }}
                onClick={() => setFolder(f.id)}
                aria-current={onHosts && folderId === f.id ? 'true' : undefined}
                className={cn(
                  row,
                  onHosts && folderId === f.id ? 'bg-muted font-medium' : 'hover:bg-muted',
                )}
              >
                <FolderIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                <span className="min-w-0 flex-1 truncate">{f.name}</span>{' '}
                <span className="text-muted-foreground text-xs">
                  {countUnder(f.id, folders, hosts)}
                </span>
              </button>
            )}
            {folderRows(f.id)}
          </li>
        ))}
      </ul>
    )
  }

  return (
    <nav aria-label="Main" className="flex w-full flex-col gap-4 overflow-y-auto bg-sidebar p-3">
      <Button className="w-full" onClick={() => openNewConnection()}>
        <Plus aria-hidden />
        New connection
      </Button>
      <div className="space-y-1">
        <button
          type="button"
          onClick={() => {
            setFolder(null)
            openPage('hosts')
          }}
          aria-current={onHosts ? 'page' : undefined}
          className={cn(row, onHosts ? 'bg-muted font-medium' : 'hover:bg-muted')}
        >
          <LayoutGrid className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          Hosts
        </button>
        <button
          type="button"
          onClick={() => openPage('tunnels')}
          aria-current={page === 'tunnels' ? 'page' : undefined}
          className={cn(row, page === 'tunnels' ? 'bg-muted font-medium' : 'hover:bg-muted')}
        >
          <Cable className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <span className="min-w-0 flex-1 truncate">Tunnels</span>
          {tunnelsOn > 0 && (
            <span className="rounded-full bg-primary px-1.5 text-primary-foreground text-xs">
              {tunnelsOn} on
            </span>
          )}
        </button>
        <button
          type="button"
          onClick={() => openPage('keys')}
          aria-current={page === 'keys' ? 'page' : undefined}
          className={cn(row, page === 'keys' ? 'bg-muted font-medium' : 'hover:bg-muted')}
        >
          <KeyRound className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          Keys & passwords
        </button>
        <button
          type="button"
          onClick={() => openPage('knownHosts')}
          aria-current={page === 'knownHosts' ? 'page' : undefined}
          className={cn(row, page === 'knownHosts' ? 'bg-muted font-medium' : 'hover:bg-muted')}
        >
          <ShieldCheck className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          Known hosts
        </button>
      </div>
      <section className="space-y-1">
        <h2 className="px-2.5 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
          Folders
        </h2>
        <button
          type="button"
          data-drop-folder=""
          onClick={() => setFolder(null)}
          aria-current={onHosts && folderId === null && !tagId ? 'true' : undefined}
          className={cn(
            row,
            onHosts && folderId === null && !tagId ? 'bg-muted font-medium' : 'hover:bg-muted',
          )}
        >
          <span className="min-w-0 flex-1 truncate">All hosts</span>{' '}
          <span className="text-muted-foreground text-xs">{hosts.length}</span>
        </button>
        {folderRows(null)}
        {creating ? (
          <InlineName
            initial=""
            label="Folder name"
            onSave={(name) => {
              setCreating(false)
              const siblings = folders.filter((f) => f.parentId === folderId).map((f) => f.order)
              saveFolder({
                id: crypto.randomUUID(),
                name,
                parentId: folderId,
                icon: { kind: 'builtin', name: 'server' },
                order: siblings.length ? Math.max(...siblings) + 1 : 0,
              }).catch(fail)
            }}
            onCancel={() => setCreating(false)}
          />
        ) : (
          <button
            type="button"
            onClick={() => setCreating(true)}
            className={cn(row, 'text-muted-foreground hover:bg-muted')}
          >
            <FolderPlus className="size-4 shrink-0" aria-hidden />
            New folder
          </button>
        )}
        {error && (
          <p role="alert" className="px-2.5 text-destructive text-xs">
            {error}
          </p>
        )}
      </section>
      {tags.length > 0 && (
        <section className="space-y-1">
          <h2 className="px-2.5 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
            Tags
          </h2>
          <ul>
            {tags.map((t) => (
              <li key={t.id}>
                <button
                  type="button"
                  onClick={() => setTag(t.id)}
                  aria-current={onHosts && tagId === t.id ? 'true' : undefined}
                  className={cn(
                    row,
                    onHosts && tagId === t.id ? 'bg-muted font-medium' : 'hover:bg-muted',
                  )}
                >
                  <span
                    className={cn(
                      'size-2 shrink-0 rounded-full',
                      t.showColor ? tagColor(t.color)?.dot : 'bg-border',
                    )}
                  />
                  {t.name}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <button
        type="button"
        onClick={() => openPage('settings')}
        aria-current={page === 'settings' ? 'page' : undefined}
        className={cn(
          row,
          'mt-auto',
          page === 'settings' ? 'bg-muted font-medium' : 'hover:bg-muted',
        )}
      >
        <Settings className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        Settings
      </button>
    </nav>
  )
}
