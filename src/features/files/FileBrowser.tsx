import { getCurrentWebview } from '@tauri-apps/api/webview'
import { confirm as confirmDialog, message } from '@tauri-apps/plugin-dialog'
import { ChevronUp, FolderPlus } from 'lucide-react'
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
import { basename } from '@/lib/sftp'
import { useTransferStore } from '@/stores/transferStore'
import { FileList } from '../sftp/FileList'
import { TransfersBar } from '../sftp/TransfersBar'

// A protocol-agnostic set of file operations. SFTP and FTP each supply one.
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
  | { kind: 'delete'; entry: SftpEntry }

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
  const progress = useTransferStore((s) => s.progress)
  const finish = useTransferStore((s) => s.finish)
  const cwdRef = useRef(cwd)
  cwdRef.current = cwd
  // The drag-drop listener is webview-wide, so every mounted panel hears every drop;
  // only the visible one should act on it.
  const activeRef = useRef(active)
  activeRef.current = active
  const [op, setOp] = useState<Op | null>(null)

  async function doDownload(entry: SftpEntry) {
    const id = crypto.randomUUID()
    start({ id, dir: 'download', name: entry.name, transferred: 0, total: 0, status: 'active' })
    try {
      const ok = await backend.download(id, entry, (p) => progress(id, p.transferred, p.total))
      finish(id)
      if (!ok) useTransferStore.getState().remove(id)
    } catch {
      finish(id, true)
    }
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
    for (const p of paths) {
      const id = crypto.randomUUID()
      start({ id, dir: 'upload', name: basename(p), transferred: 0, total: 0, status: 'active' })
      try {
        await backend.upload(id, p, cwdRef.current, (pr) => progress(id, pr.transferred, pr.total))
        finish(id)
      } catch {
        finish(id, true)
      }
    }
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
          onEnter={enter}
          onDownload={(e) => doDownload(e)}
          onRename={(e) => setOp({ kind: 'rename', entry: e })}
          onDelete={(e) => setOp({ kind: 'delete', entry: e })}
        />
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

      <Dialog
        open={op?.kind === 'delete'}
        onOpenChange={(open) => {
          if (!open) setOp(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {op?.kind === 'delete' ? op.entry.name : ''}?</DialogTitle>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setOp(null)}>
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={async () => {
                if (op?.kind !== 'delete') return
                try {
                  await backend.remove(op.entry.path, op.entry.isDir)
                  setOp(null)
                  refresh(cwdRef.current)
                } catch (e) {
                  await message(String(e), { title: 'Delete failed', kind: 'error' })
                }
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
