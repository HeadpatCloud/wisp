import { TriangleAlert } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { commands, type KnownHostEntry } from '@/bindings'
import { Button } from '@/components/ui/button'
import { PageShell } from '@/components/ui/page-shell'
import { unwrap } from '@/lib/ipc'
import { cn } from '@/lib/utils'

const NO_CERT = 'no certificate (unencrypted login)'

const KIND: Record<string, string> = {
  ssh: 'SSH host key',
  rdp: 'RDP certificate',
  rdg: 'RD Gateway certificate',
  vnc: 'VNC certificate',
}

type Chip = 'all' | 'ssh' | 'certs'
const CHIPS: [Chip, string][] = [
  ['all', 'All'],
  ['ssh', 'SSH host keys'],
  ['certs', 'Certificates'],
]

// Certificates are stored as "SHA256:<hex>"; people compare them as uppercase byte pairs.
function shownFingerprint(e: KnownHostEntry): string {
  if (e.kind === 'ssh') return e.fingerprint
  const hex = e.fingerprint.replace(/^SHA256:/, '').toUpperCase()
  return `SHA-256 ${hex.match(/../g)?.join(':') ?? hex}`
}

export function KnownHostsPage({ onClose }: { onClose: () => void }) {
  const [entries, setEntries] = useState<KnownHostEntry[]>([])
  const [query, setQuery] = useState('')
  const [chip, setChip] = useState<Chip>('all')
  const [error, setError] = useState<string | null>(null)
  const load = useCallback(() => {
    commands
      .knownHostsList()
      .then((r) => setEntries(unwrap(r)))
      .catch((e: unknown) => setError(String(e)))
  }, [])
  useEffect(load, [load])

  const q = query.trim().toLowerCase()
  const shown = entries.filter(
    (e) =>
      (chip === 'all' || (chip === 'ssh') === (e.kind === 'ssh')) &&
      (!q || `${e.host}:${e.port} ${e.fingerprint}`.toLowerCase().includes(q)),
  )
  const forget = (key: string) =>
    commands
      .knownHostsForget(key)
      .then((r) => {
        unwrap(r)
        load()
      })
      .catch((e: unknown) => setError(String(e)))

  return (
    <PageShell
      title="Known hosts"
      footer={
        <Button type="button" variant="ghost" onClick={onClose}>
          Back
        </Button>
      }
    >
      <div className="flex max-w-3xl flex-col gap-4">
        <div className="flex flex-wrap items-center gap-3">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Filter"
            aria-label="Filter known hosts"
            className="h-9 min-w-48 flex-1 rounded-md border border-input bg-background px-3 text-sm"
          />
          <fieldset className="flex gap-1">
            <legend className="sr-only">Show</legend>
            {CHIPS.map(([value, label]) => (
              <button
                key={value}
                type="button"
                aria-pressed={chip === value}
                onClick={() => setChip(value)}
                className={cn(
                  'rounded-full border border-border px-3 py-1 text-sm',
                  chip === value ? 'bg-primary text-primary-foreground' : 'hover:bg-muted',
                )}
              >
                {label}
              </button>
            ))}
          </fieldset>
        </div>
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
        {entries.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            No trusted keys yet. Wisp asks the first time it connects to a host.
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {shown.map((e) => (
              <li
                key={e.key}
                className="flex items-center gap-3 rounded-lg border border-border p-3 text-sm"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">
                    {e.host}:{e.port}
                  </p>
                  <p className="truncate text-muted-foreground text-xs">{KIND[e.kind] ?? e.kind}</p>
                  {e.fingerprint === NO_CERT ? (
                    <p className="flex items-center gap-1 text-destructive text-xs">
                      <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
                      No certificate, unencrypted login allowed
                    </p>
                  ) : (
                    <p className="break-all font-mono text-muted-foreground text-xs">
                      {shownFingerprint(e)}
                    </p>
                  )}
                </div>
                <Button type="button" variant="outline" size="sm" onClick={() => forget(e.key)}>
                  Forget
                </Button>
              </li>
            ))}
          </ul>
        )}
        <p className="text-muted-foreground text-xs">
          Wisp asks before it trusts a new key and warns when a known one changes. Forget a key to
          be asked again on the next connect.
        </p>
      </div>
    </PageShell>
  )
}
