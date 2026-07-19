import { getCurrentWebview } from '@tauri-apps/api/webview'
import { confirm as confirmDialog, message, open as openDialog } from '@tauri-apps/plugin-dialog'
import { openPath } from '@tauri-apps/plugin-opener'
import { ChevronRight, ChevronUp, Eye, EyeOff, FolderPlus, RefreshCw, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { SftpEntry, TransferProgress } from '@/bindings'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PromptDialog } from '@/components/ui/prompt-dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { formatBytes } from '@/lib/format'
import { editTempPath, fileMtime } from '@/lib/local'
import { basename } from '@/lib/sftp'
import { runTransfer } from '@/lib/transferQueue'
import { cn } from '@/lib/utils'
import { useTransferStore } from '@/stores/transferStore'
import { FileList, type SortKey } from '../sftp/FileList'
import { TransfersBar } from '../sftp/TransfersBar'

// Shareable links for an entry. `url` identifies the file; `signedUrl` (S3 only) mints a
// time-limited link that authorizes the download on its own.
export interface FileLinks {
  url(entry: SftpEntry): string
  signedUrl?(entry: SftpEntry, expiresSecs: number): Promise<string>
}

// A protocol-agnostic set of file operations. SFTP, FTP and S3 each supply one.
export interface FileBackend {
  links?: FileLinks
  list(path: string): Promise<SftpEntry[]>
  exists(path: string): Promise<boolean>
  mkdir(path: string): Promise<void>
  rename(from: string, to: string): Promise<void>
  remove(path: string, isDir: boolean): Promise<void>
  upload(
    transferId: string,
    localPath: string,
    remoteDir: string,
    onProgress: (p: TransferProgress) => void,
  ): Promise<string>
  download(
    transferId: string,
    entry: SftpEntry,
    onProgress: (p: TransferProgress) => void,
  ): Promise<boolean>
  downloadTo(
    transferId: string,
    entry: SftpEntry,
    destPath: string,
    onProgress: (p: TransferProgress) => void,
  ): Promise<void>
}

// S3 folder paths come back with a trailing slash, so strip it or keys end up with `//`.
function joinPath(dir: string, name: string): string {
  const base = dir.replace(/\/+$/, '')
  return base ? `${base}/${name}` : `/${name}`
}

// Keeps the trailing slash on absolute paths: an S3 prefix without it lists the folder
// alongside its siblings instead of listing what's inside it.
function parentOf(path: string): string {
  const parent = path.replace(/\/[^/]+\/?$/, '')
  if (!parent) return '/'
  return parent.startsWith('/') && !parent.endsWith('/') ? `${parent}/` : parent
}

const DEFAULT_EXPIRY = 3600
const EXPIRIES = [
  { value: 900, label: '15 minutes' },
  { value: DEFAULT_EXPIRY, label: '1 hour' },
  { value: 86400, label: '24 hours' },
  { value: 604800, label: '7 days' },
]

function useFileBrowser(backend: FileBackend, initial = '.') {
  const [cwd, setCwd] = useState(initial)
  const [entries, setEntries] = useState<SftpEntry[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(
    (path: string) => {
      setError(null)
      setLoading(true)
      backend
        .list(path)
        .then((e) => {
          setEntries(e)
          setCwd(path)
        })
        .catch((err) => setError(String(err)))
        .finally(() => setLoading(false))
    },
    [backend],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: load once when the backend changes
  useEffect(() => {
    refresh(initial)
  }, [refresh])

  const enter = (e: SftpEntry) => {
    if (e.isDir) refresh(e.path)
  }
  const up = () => refresh(parentOf(cwd))
  return { cwd, entries, error, loading, enter, up, refresh }
}

// Only absolute paths get clickable crumbs: SFTP/FTP browse relative to the login
// directory, where rebuilding a segment as `/a/b` would point somewhere else entirely.
// Each segment keeps a trailing slash because S3 needs it to list a prefix.
function crumbs(cwd: string): { label: string; path: string }[] {
  if (!cwd.startsWith('/')) return []
  const parts = cwd.split('/').filter(Boolean)
  return parts.map((label, i) => ({ label, path: `/${parts.slice(0, i + 1).join('/')}/` }))
}

type Op =
  | { kind: 'mkdir' }
  | { kind: 'delete'; entries: SftpEntry[] }
  | { kind: 'move'; entries: SftpEntry[] }

export function FileBrowser({
  backend,
  active = true,
  initialPath = '.',
}: {
  backend: FileBackend
  active?: boolean
  initialPath?: string
}) {
  const { cwd, entries, error, loading, up, enter, refresh } = useFileBrowser(backend, initialPath)
  const start = useTransferStore((s) => s.start)
  const activate = useTransferStore((s) => s.activate)
  const progress = useTransferStore((s) => s.progress)
  const finish = useTransferStore((s) => s.finish)
  const cwdRef = useRef(cwd)
  cwdRef.current = cwd
  // The drag-drop listener is webview-wide, so every mounted panel hears every drop;
  // only the visible one should act on it.
  const activeRef = useRef(active)
  activeRef.current = active
  const [op, setOp] = useState<Op | null>(null)
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const anchorRef = useRef<string | null>(null)
  const entriesRef = useRef(entries)
  const selectedRef = useRef(selected)
  selectedRef.current = selected
  const opRef = useRef(op)
  opRef.current = op
  const [bulk, setBulk] = useState<{
    verb: string
    done: number
    total: number
    failed: number
    status: 'running' | 'cancelling' | 'stopped' | 'failed'
    error?: string
  } | null>(null)
  const cancelBulkRef = useRef(false)
  const bulkRef = useRef(bulk)
  bulkRef.current = bulk
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'name', desc: false })
  const [filter, setFilter] = useState('')
  const [showHidden, setShowHidden] = useState(false)
  const [renaming, setRenaming] = useState<string | null>(null)
  const actionsRef = useRef<{
    enter: (e: SftpEntry) => void
    up: () => void
    download: (e: SftpEntry) => void
  } | null>(null)
  const editTimers = useRef<number[]>([])
  const editing = useRef(new Set<string>())
  const disposedRef = useRef(false)
  const [link, setLink] = useState<{
    title: string
    url: string
    entry: SftpEntry
    expires: number
  } | null>(null)
  const linkRef = useRef(link)
  linkRef.current = link
  // Only keyboard navigation should scroll the list; a click is already in view.
  const keyNavRef = useRef(false)

  const q = filter.trim().toLowerCase()
  const visible = entries
    .filter((e) => showHidden || !e.name.startsWith('.'))
    .filter((e) => !q || e.name.toLowerCase().includes(q))
    .slice()
    .sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
      const n =
        sort.key === 'name'
          ? a.name.localeCompare(b.name)
          : sort.key === 'size'
            ? a.size - b.size
            : (a.modified ?? 0) - (b.modified ?? 0)
      return sort.desc ? -n : n
    })

  // Keyboard selection acts on what's on screen, so it follows the filter and sort order.
  entriesRef.current = visible

  const selectedEntries = entries.filter((e) => selected.has(e.path))
  const selectedSize = selectedEntries.reduce((n, e) => n + (e.isDir ? 0 : e.size), 0)
  // Deleting by key must cover the whole selection, not just rows the filter leaves on screen.
  const selectedEntriesRef = useRef(selectedEntries)
  selectedEntriesRef.current = selectedEntries

  // A context-menu action on a row inside a multi-selection applies to the whole selection.
  const targetsFor = (e: SftpEntry) =>
    selected.has(e.path) && selectedEntries.length > 1 ? selectedEntries : [e]

  const copyPaths = (list: SftpEntry[]) =>
    navigator.clipboard.writeText(list.map((e) => e.path).join('\n')).catch(() => {})

  const showUrl = (e: SftpEntry) => {
    const url = backend.links?.url(e)
    if (url) setLink({ title: e.name, url, entry: e, expires: 0 })
  }

  const showSignedLink = async (e: SftpEntry, expires: number) => {
    const gen = backend.links?.signedUrl
    if (!gen) return
    try {
      setLink({ title: e.name, url: await gen(e, expires), entry: e, expires })
    } catch (err) {
      await message(String(err), { title: 'Could not create link', kind: 'error' })
    }
  }

  // Keep selection valid: navigating away or deleting drops gone paths, and hiding dotfiles
  // must drop them too or they'd be deleted/moved while invisible.
  useEffect(() => {
    setSelected((prev) => {
      const paths = new Set(
        entries.filter((e) => showHidden || !e.name.startsWith('.')).map((e) => e.path),
      )
      const next = new Set([...prev].filter((p) => paths.has(p)))
      return next.size === prev.size ? prev : next
    })
  }, [entries, showHidden])

  function rowClick(entry: SftpEntry, mods: { toggle: boolean; range: boolean }) {
    setSelected((prev) => {
      if (mods.range && anchorRef.current) {
        const a = visible.findIndex((e) => e.path === anchorRef.current)
        const b = visible.findIndex((e) => e.path === entry.path)
        if (a !== -1 && b !== -1) {
          const [lo, hi] = a < b ? [a, b] : [b, a]
          const range = visible.slice(lo, hi + 1).map((e) => e.path)
          return mods.toggle ? new Set([...prev, ...range]) : new Set(range)
        }
      }
      if (mods.toggle) {
        const next = new Set(prev)
        if (next.has(entry.path)) next.delete(entry.path)
        else next.add(entry.path)
        return next
      }
      return new Set([entry.path])
    })
    if (!mods.range) anchorRef.current = entry.path
  }

  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (!activeRef.current || opRef.current || bulkRef.current || linkRef.current) return
      // Radix menus/selects preventDefault the keys they consume without stopping
      // propagation, so this is what keeps an open context menu from double-acting.
      if (ev.defaultPrevented) return
      const t = ev.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      if (t?.closest('[role="menu"],[role="dialog"],[role="listbox"]')) return
      if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 'a') {
        ev.preventDefault()
        setSelected(new Set(entriesRef.current.map((e) => e.path)))
      } else if (ev.key === 'Escape' && selectedRef.current.size > 0) {
        setSelected(new Set())
      } else if (ev.key === 'Delete' && selectedRef.current.size > 0) {
        setOp({ kind: 'delete', entries: selectedEntriesRef.current })
      } else if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
        const list = entriesRef.current
        if (list.length === 0) return
        ev.preventDefault()
        // Track the cursor from the anchor, not the topmost selected row, or arrows jump
        // to the start of a multi-selection.
        const cur = anchorRef.current
          ? list.findIndex((e) => e.path === anchorRef.current)
          : list.findIndex((e) => selectedRef.current.has(e.path))
        const next =
          cur === -1
            ? 0
            : ev.key === 'ArrowDown'
              ? Math.min(cur + 1, list.length - 1)
              : Math.max(cur - 1, 0)
        setSelected(new Set([list[next].path]))
        anchorRef.current = list[next].path
        keyNavRef.current = true
      } else if (ev.key === 'Enter' && selectedRef.current.size === 1) {
        const e = entriesRef.current.find((x) => selectedRef.current.has(x.path))
        if (!e) return
        ev.preventDefault()
        if (e.isDir) actionsRef.current?.enter(e)
        else actionsRef.current?.download(e)
      } else if (ev.key === 'Backspace') {
        ev.preventDefault()
        actionsRef.current?.up()
      } else if (ev.key === 'F2' && selectedRef.current.size === 1) {
        ev.preventDefault()
        setRenaming([...selectedRef.current][0])
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Stop an in-flight bulk loop when the panel unmounts (SFTP toggle, pane close, disconnect).
  useEffect(
    () => () => {
      cancelBulkRef.current = true
    },
    [],
  )

  function doDownload(entry: SftpEntry) {
    const id = crypto.randomUUID()
    start({ id, dir: 'download', name: entry.name, transferred: 0, total: 0, status: 'queued' })
    runTransfer(id, async () => {
      activate(id)
      try {
        const ok = await backend.download(id, entry, (p) => progress(id, p.transferred, p.total))
        finish(id)
        if (!ok) useTransferStore.getState().remove(id)
      } catch {
        finish(id, true)
      }
    })
  }

  actionsRef.current = { enter, up, download: doDownload }

  // Round-trip through a local temp copy: download, hand it to the OS editor, then poll the
  // file's mtime and re-upload whenever it's saved. Polling avoids a native watcher.
  async function openInEditor(entry: SftpEntry) {
    if (editing.current.has(entry.path)) return
    editing.current.add(entry.path)
    const local = await editTempPath(entry.name)
    const id = crypto.randomUUID()
    start({ id, dir: 'download', name: entry.name, transferred: 0, total: 0, status: 'queued' })
    runTransfer(id, async () => {
      activate(id)
      try {
        await backend.downloadTo(id, entry, local, (p) => progress(id, p.transferred, p.total))
        finish(id)
      } catch {
        finish(id, true)
        editing.current.delete(entry.path)
        return
      }
      try {
        await openPath(local)
      } catch (e) {
        editing.current.delete(entry.path)
        await message(String(e), { title: 'Could not open the editor', kind: 'error' })
        return
      }
      // The panel may already be gone by the time the download finishes, in which case the
      // unmount cleanup has run and would never see a timer registered now.
      if (disposedRef.current) {
        editing.current.delete(entry.path)
        return
      }
      let last = await fileMtime(local).catch(() => 0)
      let uploading = false
      const timer = window.setInterval(async () => {
        if (uploading) return
        const now = await fileMtime(local).catch(() => 0)
        if (!now || now === last) return
        uploading = true
        const upId = crypto.randomUUID()
        start({
          id: upId,
          dir: 'upload',
          name: entry.name,
          transferred: 0,
          total: 0,
          status: 'queued',
        })
        runTransfer(upId, async () => {
          activate(upId)
          try {
            await backend.upload(upId, local, parentOf(entry.path), (p) =>
              progress(upId, p.transferred, p.total),
            )
            finish(upId)
            // Only advance past this save once it actually landed, so a failed upload
            // is retried on the next tick instead of dropping the edit.
            last = now
            refresh(cwdRef.current)
          } catch {
            finish(upId, true)
          } finally {
            uploading = false
          }
        })
      }, 2000)
      editTimers.current.push(timer)
    })
  }

  useEffect(() => {
    const timers = editTimers.current
    const disposed = disposedRef
    return () => {
      disposed.current = true
      for (const t of timers) window.clearInterval(t)
    }
  }, [])

  // One file keeps the save-as dialog; several download into a single picked directory.
  async function downloadMany(list: SftpEntry[]) {
    if (list.length === 1) {
      doDownload(list[0])
      return
    }
    const parent = await openDialog({ directory: true })
    if (typeof parent !== 'string') return
    for (const entry of list) {
      const id = crypto.randomUUID()
      start({ id, dir: 'download', name: entry.name, transferred: 0, total: 0, status: 'queued' })
      runTransfer(id, async () => {
        activate(id)
        try {
          await backend.downloadTo(id, entry, `${parent}/${entry.name}`, (p) =>
            progress(id, p.transferred, p.total),
          )
          finish(id)
        } catch {
          finish(id, true)
        }
      })
    }
  }

  async function runBulk(verb: string, list: SftpEntry[], each: (e: SftpEntry) => Promise<void>) {
    setOp(null)
    cancelBulkRef.current = false
    setBulk({ verb, done: 0, total: list.length, failed: 0, status: 'running' })
    let failed = 0
    let firstError: string | undefined
    let cancelled = false
    for (let i = 0; i < list.length; i++) {
      // Only checked between items, so a cancel during the last item still finishes it and
      // completes normally - the loop exits via the condition, not this break.
      if (cancelBulkRef.current) {
        cancelled = true
        break
      }
      try {
        await each(list[i])
      } catch (e) {
        failed++
        if (!firstError) {
          firstError =
            e && typeof e === 'object' && 'message' in e
              ? String((e as { message: unknown }).message)
              : String(e)
        }
      }
      setBulk((b) => (b ? { ...b, done: i + 1, failed } : b))
    }
    setSelected(new Set())
    if (!cancelled && failed === 0) setBulk(null)
    else {
      setBulk((b) =>
        b ? { ...b, status: cancelled ? 'stopped' : 'failed', error: firstError } : b,
      )
    }
    refresh(cwdRef.current)
  }

  const removeMany = (list: SftpEntry[]) =>
    runBulk('Deleting', list, (e) => backend.remove(e.path, e.isDir))

  function moveMany(list: SftpEntry[], dest: string) {
    const dir = dest !== '/' ? dest.replace(/\/+$/, '') : dest
    return runBulk('Moving', list, (e) => backend.rename(e.path, joinPath(dir, e.name)))
  }

  async function uploadPaths(paths: string[]) {
    const conflicts: string[] = []
    for (const p of paths) {
      if (await backend.exists(joinPath(cwdRef.current, basename(p)))) conflicts.push(basename(p))
    }
    if (conflicts.length > 0) {
      const ok = await confirmDialog(
        `${conflicts.length} item(s) already exist and will be overwritten. Continue?`,
        { title: 'Overwrite?' },
      )
      if (!ok) return
    }
    const dir = cwdRef.current
    const runs = paths.map((p) => {
      const id = crypto.randomUUID()
      start({ id, dir: 'upload', name: basename(p), transferred: 0, total: 0, status: 'queued' })
      return runTransfer(id, async () => {
        activate(id)
        try {
          await backend.upload(id, p, dir, (pr) => progress(id, pr.transferred, pr.total))
          finish(id)
        } catch {
          finish(id, true)
        }
      })
    })
    await Promise.allSettled(runs)
    refresh(cwdRef.current)
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: listener bound for the panel's lifetime
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && activeRef.current) uploadPaths(event.payload.paths)
      })
      .then((u) => {
        if (disposed) u()
        else unlisten = u
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [backend])

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-1 border-border border-b p-1">
        <button type="button" aria-label="Up" onClick={up} className="rounded p-1 hover:bg-muted">
          <ChevronUp className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Refresh"
          onClick={() => refresh(cwdRef.current)}
          className="rounded p-1 hover:bg-muted"
        >
          <RefreshCw className="size-4" />
        </button>
        <button
          type="button"
          aria-label="New folder"
          onClick={() => setOp({ kind: 'mkdir' })}
          className="rounded p-1 hover:bg-muted"
        >
          <FolderPlus className="size-4" />
        </button>
        <button
          type="button"
          aria-label={showHidden ? 'Hide hidden files' : 'Show hidden files'}
          onClick={() => setShowHidden((v) => !v)}
          className={cn('rounded p-1 hover:bg-muted', showHidden && 'text-primary')}
        >
          {showHidden ? <Eye className="size-4" /> : <EyeOff className="size-4" />}
        </button>
        <nav className="flex min-w-0 flex-1 items-center gap-0.5 overflow-hidden text-muted-foreground text-xs">
          {cwd.startsWith('/') ? (
            <button
              type="button"
              onClick={() => refresh('/')}
              className="shrink-0 rounded px-1 py-0.5 hover:bg-muted hover:text-foreground"
            >
              /
            </button>
          ) : (
            <span className="truncate px-1">{cwd}</span>
          )}
          {crumbs(cwd).map((c, i, all) => (
            <span key={c.path} className="flex min-w-0 items-center gap-0.5">
              <ChevronRight className="size-3 shrink-0" />
              <button
                type="button"
                onClick={() => refresh(c.path)}
                className={cn(
                  'truncate rounded px-1 py-0.5 hover:bg-muted hover:text-foreground',
                  i === all.length - 1 && 'text-foreground',
                )}
              >
                {c.label}
              </button>
            </span>
          ))}
        </nav>
        <Input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter…"
          className="h-7 w-36 shrink-0 text-xs"
        />
      </div>
      {error ? (
        <div className="p-2 text-destructive text-xs">{error}</div>
      ) : loading && entries.length === 0 ? (
        <p className="p-6 text-center text-muted-foreground text-sm">Loading…</p>
      ) : visible.length === 0 ? (
        <p className="p-6 text-center text-muted-foreground text-sm">
          {entries.length === 0 ? 'This folder is empty.' : 'No matching files.'}
        </p>
      ) : (
        <FileList
          entries={visible}
          selected={selected}
          sort={sort}
          onSort={(k) => setSort((s) => ({ key: k, desc: s.key === k ? !s.desc : false }))}
          keyNav={keyNavRef}
          onRowClick={(e, mods) => {
            keyNavRef.current = false
            rowClick(e, mods)
          }}
          onEnter={enter}
          onDownload={(e) => downloadMany(targetsFor(e))}
          onRename={(e) => setRenaming(e.path)}
          onEdit={(e) => openInEditor(e)}
          renaming={renaming}
          onRenameCancel={() => setRenaming(null)}
          onRenameSubmit={async (e, name) => {
            setRenaming(null)
            try {
              await backend.rename(e.path, joinPath(parentOf(e.path), name))
              refresh(cwdRef.current)
            } catch (err) {
              await message(String(err), { title: 'Rename failed', kind: 'error' })
            }
          }}
          onDelete={(e) => setOp({ kind: 'delete', entries: targetsFor(e) })}
          onCopyPath={(e) => copyPaths(targetsFor(e))}
          onCopyName={(e) => navigator.clipboard.writeText(e.name).catch(() => {})}
          onCopyUrl={backend.links ? showUrl : undefined}
          onSignedLink={
            backend.links?.signedUrl ? (e) => showSignedLink(e, DEFAULT_EXPIRY) : undefined
          }
        />
      )}
      {!error && selectedEntries.length > 0 && (
        <div className="fade-in slide-in-from-bottom-2 flex shrink-0 animate-in items-center gap-1 border-border border-t p-1 text-xs duration-200">
          <span className="truncate px-1 text-muted-foreground">
            {selectedEntries.length} selected
            {selectedSize > 0 ? ` · ${formatBytes(selectedSize)}` : ''}
          </span>
          <span className="min-w-0 flex-1" />
          <button
            type="button"
            onClick={() => downloadMany(selectedEntries)}
            className="rounded px-2 py-1 hover:bg-muted"
          >
            Download
          </button>
          <button
            type="button"
            onClick={() => setOp({ kind: 'move', entries: selectedEntries })}
            className="rounded px-2 py-1 hover:bg-muted"
          >
            Move…
          </button>
          <button
            type="button"
            onClick={() => copyPaths(selectedEntries)}
            className="rounded px-2 py-1 hover:bg-muted"
          >
            Copy paths
          </button>
          <button
            type="button"
            onClick={() => setOp({ kind: 'delete', entries: selectedEntries })}
            className="rounded px-2 py-1 text-destructive hover:bg-muted"
          >
            Delete
          </button>
          <button
            type="button"
            aria-label="Clear selection"
            onClick={() => setSelected(new Set())}
            className="rounded p-1 hover:bg-muted"
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}
      <TransfersBar />

      <PromptDialog
        open={op?.kind === 'mkdir'}
        title="New folder"
        onConfirm={async (name) => {
          try {
            await backend.mkdir(joinPath(cwdRef.current, name))
            setOp(null)
            refresh(cwdRef.current)
          } catch (e) {
            await message(String(e), { title: 'Create folder failed', kind: 'error' })
          }
        }}
        onOpenChange={(open) => {
          if (!open) setOp(null)
        }}
      />

      <PromptDialog
        open={op?.kind === 'move'}
        title={
          op?.kind === 'move'
            ? `Move ${op.entries.length} item${op.entries.length === 1 ? '' : 's'} to`
            : ''
        }
        defaultValue={cwd}
        confirmLabel="Move"
        onConfirm={(dest) => {
          if (op?.kind !== 'move') return
          moveMany(op.entries, dest)
        }}
        onOpenChange={(open) => {
          if (!open) setOp(null)
        }}
      />

      <Dialog
        open={op?.kind === 'delete'}
        onOpenChange={(open) => {
          if (!open) setOp(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {op?.kind === 'delete'
                ? op.entries.length === 1
                  ? `Delete ${op.entries[0].name}?`
                  : `Delete ${op.entries.length} items?`
                : ''}
            </DialogTitle>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setOp(null)}>
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => {
                if (op?.kind !== 'delete') return
                removeMany(op.entries)
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={link !== null}
        onOpenChange={(open) => {
          if (!open) setLink(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="truncate">{link?.title}</DialogTitle>
          </DialogHeader>
          {link && link.expires > 0 && (
            <div className="space-y-1">
              <Label htmlFor="link-expiry">Expires in</Label>
              <Select
                value={String(link.expires)}
                onValueChange={(v) => showSignedLink(link.entry, Number(v))}
              >
                <SelectTrigger id="link-expiry">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {EXPIRIES.map((x) => (
                    <SelectItem key={x.value} value={String(x.value)}>
                      {x.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <Input
            readOnly
            value={link?.url ?? ''}
            onFocus={(ev) => ev.currentTarget.select()}
            className="text-xs"
          />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setLink(null)}>
              Close
            </Button>
            <Button
              type="button"
              onClick={() => {
                if (link) navigator.clipboard.writeText(link.url).catch(() => {})
                setLink(null)
              }}
            >
              Copy
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={bulk !== null}
        onOpenChange={(open) => {
          if (!open && bulk?.status !== 'running' && bulk?.status !== 'cancelling') setBulk(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {bulk?.status === 'running'
                ? `${bulk.verb} ${bulk.done} of ${bulk.total}…`
                : bulk?.status === 'cancelling'
                  ? `Cancelling… (${bulk.done} of ${bulk.total})`
                  : bulk?.status === 'stopped'
                    ? `Stopped at ${bulk.done} of ${bulk.total}${bulk.failed ? ` · ${bulk.failed} failed` : ''}`
                    : bulk
                      ? `${bulk.failed} of ${bulk.total} failed`
                      : ''}
            </DialogTitle>
          </DialogHeader>
          <div className="h-1 rounded bg-muted">
            <div
              className="h-1 rounded bg-primary transition-[width]"
              style={{
                width: `${bulk?.total ? Math.round((bulk.done / bulk.total) * 100) : 0}%`,
              }}
            />
          </div>
          {bulk?.error ? <p className="text-destructive text-xs">{bulk.error}</p> : null}
          <DialogFooter>
            {bulk?.status === 'running' || bulk?.status === 'cancelling' ? (
              <Button
                type="button"
                variant="ghost"
                disabled={bulk.status === 'cancelling'}
                onClick={() => {
                  cancelBulkRef.current = true
                  setBulk((b) => (b ? { ...b, status: 'cancelling' } : b))
                }}
              >
                {bulk.status === 'cancelling' ? 'Cancelling…' : 'Cancel'}
              </Button>
            ) : (
              <Button type="button" variant="ghost" onClick={() => setBulk(null)}>
                Close
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
