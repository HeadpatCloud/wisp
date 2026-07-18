import { useEffect, useMemo, useRef, useState } from 'react'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ProfileIcon } from '@/features/profiles/ProfileIcon'
import { cn } from '@/lib/utils'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'

interface Item {
  id: string
  label: string
  hint?: string
  group: string
  icon?: React.ReactNode
  run: () => void
}

// Subsequence match: "wb1" finds "web-01". Returns null when it doesn't match.
function score(query: string, text: string): number | null {
  if (!query) return 0
  const t = text.toLowerCase()
  let i = 0
  let first = -1
  for (const ch of query) {
    const at = t.indexOf(ch, i)
    if (at === -1) return null
    if (first === -1) first = at
    i = at + 1
  }
  return first
}

export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const profiles = useProfileStore((s) => s.profiles)
  const s3Profiles = useS3ProfileStore((s) => s.profiles)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (open) {
      setQuery('')
      setActive(0)
    }
  }, [open])

  const items = useMemo<Item[]>(() => {
    const st = useSessionStore.getState()
    const out: Item[] = []
    for (const p of profiles) {
      out.push({
        id: `ssh:${p.id}`,
        label: p.name,
        hint: `${p.username}@${p.host}`,
        group: 'Connect',
        icon: <ProfileIcon icon={p.icon} className="size-4 text-muted-foreground" />,
        run: () =>
          st.openTab({
            id: crypto.randomUUID(),
            profileId: p.id,
            title: p.name,
            status: 'connecting',
            reconnectNonce: 0,
          }),
      })
      out.push({
        id: `sftp:${p.id}`,
        label: `SFTP: ${p.name}`,
        hint: p.host,
        group: 'Files',
        run: () => st.openSftp(p.id, p.name),
      })
    }
    for (const p of s3Profiles) {
      out.push({
        id: `s3:${p.id}`,
        label: `S3: ${p.name}`,
        hint: p.endpoint,
        group: 'Files',
        run: () => st.openS3(p.id, p.bucket, p.name),
      })
    }
    out.push(
      {
        id: 'act:local',
        label: 'New local shell',
        group: 'Actions',
        run: () => st.openLocalShell(),
      },
      {
        id: 'act:profile',
        label: 'New SSH profile',
        group: 'Actions',
        run: () => st.openView({ kind: 'profile-editor', profileId: null }, 'New profile'),
      },
      {
        id: 'act:group',
        label: 'New group',
        group: 'Actions',
        run: () => st.openView({ kind: 'group-editor', groupId: null }, 'New group'),
      },
      {
        id: 'act:import',
        label: 'Import from SSH config',
        group: 'Actions',
        run: () => st.openView({ kind: 'import' }, 'Import'),
      },
      {
        id: 'act:settings',
        label: 'Settings',
        group: 'Actions',
        run: () => st.openView({ kind: 'settings' }, 'Settings'),
      },
    )
    return out
  }, [profiles, s3Profiles])

  const q = query.trim().toLowerCase()
  const results = useMemo(() => {
    const scored = items
      .map((it) => ({ it, s: score(q, `${it.label} ${it.hint ?? ''}`) }))
      .filter((x): x is { it: Item; s: number } => x.s !== null)
    scored.sort((a, b) => a.s - b.s)
    return scored.slice(0, 50).map((x) => x.it)
  }, [items, q])

  const clamped = Math.min(active, Math.max(0, results.length - 1))

  // biome-ignore lint/correctness/useExhaustiveDependencies: follows the highlighted row
  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [clamped])

  const run = (item: Item) => {
    onOpenChange(false)
    item.run()
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-24 translate-y-0 gap-2 p-2">
        <DialogHeader className="sr-only">
          <DialogTitle>Command palette</DialogTitle>
        </DialogHeader>
        <Input
          autoFocus
          value={query}
          placeholder="Search profiles and actions…"
          onChange={(e) => {
            setQuery(e.target.value)
            setActive(0)
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              setActive((a) => Math.min(a + 1, results.length - 1))
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              setActive((a) => Math.max(a - 1, 0))
            } else if (e.key === 'Enter' && results[clamped]) {
              e.preventDefault()
              run(results[clamped])
            }
          }}
        />
        <div ref={listRef} className="max-h-80 overflow-y-auto">
          {results.length === 0 ? (
            <p className="px-2 py-6 text-center text-muted-foreground text-sm">No matches.</p>
          ) : (
            results.map((it, i) => (
              <button
                key={it.id}
                type="button"
                data-active={i === clamped}
                onMouseEnter={() => setActive(i)}
                onClick={() => run(it)}
                className={cn(
                  'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm',
                  i === clamped && 'bg-muted',
                )}
              >
                {it.icon}
                <span className="min-w-0 flex-1 truncate">{it.label}</span>
                {it.hint && (
                  <span className="shrink-0 truncate text-muted-foreground text-xs">{it.hint}</span>
                )}
                <span className="shrink-0 text-muted-foreground text-xs">{it.group}</span>
              </button>
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
