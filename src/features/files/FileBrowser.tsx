import { getCurrentWebview } from '@tauri-apps/api/webview'
import { confirm as confirmDialog, message, open as openDialog } from '@tauri-apps/plugin-dialog'
import { ChevronUp, FolderPlus, X } from 'lucide-react'
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
import { PromptDialog } from '@/components/ui/prompt-dialog'
import { formatBytes } from '@/lib/format'
import { basename } from '@/lib/sftp'
import { runTransfer } from '@/lib/transferQueue'
import { useTransferStore } from '@/stores/transferStore'
import { FileList } from '../sftp/FileList'
import { TransfersBar } from '../sftp/TransfersBar'

// A protocol-agnostic set of file operations. SFTP, FTP and S3 each supply one.
export interface FileBackend {
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

function joinPath(dir: string, name: string): string {
  return dir === '/' ? `/${name}` : `${dir}/${name}`
}

function parentOf(path: string): string {
  return path.replace(/\/[^/]+\/?$/, '') || '/'
}

function useFileBrowser(backend: FileBackend, initial = '.') {
  const [cwd, setCwd] = useState(initial)
  const [entries, setEntries] = useState<SftpEntry[]>([])
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(
    (path: string) => {
      setError(null)
      backend
        .list(path)
        .then((e) => {
          setEntries(e)
          setCwd(path)
        })
        .catch((err) => setError(String(err)))
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
  return { cwd, entries, error, enter, up, refresh }
}

type Op =
  | { kind: 'mkdir' }
  | { kind: 'rename'; entry: SftpEntry }
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
  const { cwd, entries, error, up, enter, refresh } = useFileBrowser(backend, initialPath)
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
  entriesRef.current = entries
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

  const selectedEntries = entries.filter((e) => selected.has(e.path))
  const selectedSize = selectedEntries.reduce((n, e) => n + (e.isDir ? 0 : e.size), 0)

  // Keep selection valid across refreshes: navigating away or deleting drops gone paths.
  useEffect(() => {
    setSelected((prev) => {
      const paths = new Set(entries.map((e) => e.path))
      const next = new Set([...prev].filter((p) => paths.has(p)))
      return next.size === prev.size ? prev : next
    })
  }, [entries])

  function rowClick(entry: SftpEntry, mods: { toggle: boolean; range: boolean }) {
    setSelected((prev) => {
      if (mods.range && anchorRef.current) {
        const a = entries.findIndex((e) => e.path === anchorRef.current)
        const b = entries.findIndex((e) => e.path === entry.path)
        if (a !== -1 && b !== -1) {
          const [lo, hi] = a < b ? [a, b] : [b, a]
          const range = entries.slice(lo, hi + 1).map((e) => e.path)
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
      if (!activeRef.current || opRef.current || bulkRef.current) return
      const t = ev.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 'a') {
        ev.preventDefault()
        setSelected(new Set(entriesRef.current.map((e) => e.path)))
      } else if (ev.key === 'Escape' && selectedRef.current.size > 0) {
        setSelected(new Set())
      } else if (ev.key === 'Delete' && selectedRef.current.size > 0) {
        setOp({
          kind: 'delete',
          entries: entriesRef.current.filter((e) => selectedRef.current.has(e.path)),
        })
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
          aria-label="New folder"
          onClick={() => setOp({ kind: 'mkdir' })}
          className="rounded p-1 hover:bg-muted"
        >
          <FolderPlus className="size-4" />
        </button>
        <span className="truncate text-muted-foreground text-xs">{cwd}</span>
      </div>
      {error ? (
        <div className="p-2 text-red-600 text-xs">{error}</div>
      ) : (
        <FileList
          entries={entries}
          selected={selected}
          onRowClick={rowClick}
          onEnter={enter}
          onDownload={(e) => doDownload(e)}
          onRename={(e) => setOp({ kind: 'rename', entry: e })}
          onDelete={(e) => setOp({ kind: 'delete', entries: [e] })}
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
            onClick={() =>
              navigator.clipboard
                .writeText(selectedEntries.map((e) => e.path).join('\n'))
                .catch(() => {})
            }
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
        open={op?.kind === 'rename'}
        title="Rename"
        defaultValue={op?.kind === 'rename' ? op.entry.name : ''}
        onConfirm={async (name) => {
          if (op?.kind !== 'rename') return
          try {
            await backend.rename(op.entry.path, joinPath(parentOf(op.entry.path), name))
            setOp(null)
            refresh(cwdRef.current)
          } catch (e) {
            await message(String(e), { title: 'Rename failed', kind: 'error' })
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
