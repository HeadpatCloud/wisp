import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { useEffect, useState } from 'react'
import { commands, type ImportCandidate } from '@/bindings'
import { Button } from '@/components/ui/button'
import { PageShell } from '@/components/ui/page-shell'
import { unwrap } from '@/lib/ipc'
import { useHostStore } from '@/stores/hostStore'
import { notify } from '@/stores/noticeStore'

const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`
const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

function noteOf(c: ImportCandidate): string | null {
  if (c.status.kind === 'addsTo') return `adds to ${c.status.hostName}`
  if (c.status.kind === 'alreadyThere') return `already in ${c.status.hostName}`
  return null
}

export function SshImportPage({ onClose }: { onClose: () => void }) {
  const [path, setPath] = useState<string | null>(null)
  const [candidates, setCandidates] = useState<ImportCandidate[] | null>(null)
  const [unticked, setUnticked] = useState<Set<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let live = true
    setCandidates(null)
    setError(null)
    setUnticked(new Set())
    commands
      .importSshConfig(path)
      .then(unwrap)
      .then((list) => live && setCandidates(list))
      .catch((e: unknown) => live && setError(message(e)))
    return () => {
      live = false
    }
  }, [path])

  const ticked = (candidates ?? []).filter(
    (c) => c.status.kind !== 'alreadyThere' && !unticked.has(c.name),
  )

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      // Already-there entries go along so a ProxyJump to them links; the backend doesn't import them.
      const there = (candidates ?? []).filter((c) => c.status.kind === 'alreadyThere')
      const done = unwrap(await commands.sshConfigApply([...ticked, ...there]))
      await useHostStore.getState().load()
      notify({
        title: `Imported ${count(done.hosts, 'host')} and ${count(done.connections, 'connection')}.`,
      })
      onClose()
    } catch (e) {
      setError(message(e))
      setBusy(false)
    }
  }

  return (
    <PageShell
      title="Import from ~/.ssh/config"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={busy || ticked.length === 0} onClick={() => void run()}>
            Import {ticked.length}
          </Button>
        </>
      }
    >
      <div className="mx-auto flex max-w-2xl flex-col gap-3">
        <div className="flex items-center gap-2 text-sm">
          <span className="min-w-0 flex-1 truncate text-muted-foreground">
            {path ?? '~/.ssh/config'}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={async () => {
              const picked = await pickFile({ multiple: false, directory: false })
              if (typeof picked === 'string') setPath(picked)
            }}
          >
            Choose another file…
          </Button>
        </div>
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
        {candidates?.length === 0 && <p className="text-sm">No hosts in this file.</p>}
        <ul className="flex flex-col divide-y divide-border rounded-lg border border-border">
          {(candidates ?? []).map((c) => {
            const there = c.status.kind === 'alreadyThere'
            const note = noteOf(c)
            return (
              <li key={c.name} className="flex items-center gap-3 px-3 py-2 text-sm">
                <input
                  type="checkbox"
                  aria-label={c.name}
                  disabled={there}
                  checked={!there && !unticked.has(c.name)}
                  onChange={(e) =>
                    setUnticked((prev) => {
                      const next = new Set(prev)
                      if (e.target.checked) next.delete(c.name)
                      else next.add(c.name)
                      return next
                    })
                  }
                />
                <span className="font-medium">{c.name}</span>
                <span className="min-w-0 flex-1 truncate text-muted-foreground">
                  {c.username ? `${c.username}@` : ''}
                  {c.host}
                  {c.port !== 22 ? `:${c.port}` : ''}
                </span>
                {note && <span className="text-muted-foreground text-xs">{note}</span>}
              </li>
            )
          })}
        </ul>
      </div>
    </PageShell>
  )
}
