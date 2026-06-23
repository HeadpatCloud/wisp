import { useEffect, useState } from 'react'
import type { ImportCandidate, Profile } from '@/bindings'
import { Button } from '@/components/ui/button'
import { PageShell } from '@/components/ui/page-shell'
import { importSshConfig, nextProfileOrder } from '@/lib/profiles'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'

export function ImportPage({ tabId }: { tabId: string }) {
  const saveProfile = useProfileStore((s) => s.saveProfile)
  const existing = useProfileStore((s) => s.profiles)
  const removeTab = useSessionStore((s) => s.removeTab)
  const [candidates, setCandidates] = useState<ImportCandidate[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Record<string, boolean>>({})

  useEffect(() => {
    let alive = true
    importSshConfig()
      .then((c) => {
        if (alive) setCandidates(c)
      })
      .catch(() => {
        if (alive) setError('Could not read the SSH config file.')
      })
    return () => {
      alive = false
    }
  }, [])

  const list = candidates ?? []
  const isOn = (name: string) => selected[name] ?? true

  async function doImport() {
    const chosen = list.filter((c) => isOn(c.name))
    const withIds = chosen.map((c) => ({ candidate: c, id: crypto.randomUUID() }))
    const nameToId = new Map<string, string>()
    for (const p of existing) nameToId.set(p.name, p.id)
    for (const w of withIds) nameToId.set(w.candidate.name, w.id)
    const baseOrder = nextProfileOrder(existing, null)
    for (const [i, { candidate: c, id }] of withIds.entries()) {
      const profile: Profile = {
        id,
        name: c.name,
        groupId: null,
        host: c.host,
        port: c.port,
        username: c.username,
        authMethod: c.keyPath ? 'key' : 'password',
        keyPath: c.keyPath,
        secretId: null,
        icon: { kind: 'builtin', name: 'server' },
        order: baseOrder + i,
        jumpHostId: c.jumpHostAlias ? (nameToId.get(c.jumpHostAlias) ?? null) : null,
        tunnels: [],
      }
      await saveProfile(profile)
    }
    removeTab(tabId)
  }

  return (
    <PageShell
      title="Import from SSH config"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
            Cancel
          </Button>
          <Button type="button" disabled={list.length === 0} onClick={doImport}>
            Import
          </Button>
        </>
      }
    >
      {error ? (
        <p className="text-red-600 text-sm">{error}</p>
      ) : candidates === null ? (
        <p className="text-muted-foreground text-sm">Reading SSH config...</p>
      ) : list.length === 0 ? (
        <p className="text-muted-foreground text-sm">No hosts found in the config file.</p>
      ) : (
        <ul className="space-y-1 text-sm">
          {list.map((c) => (
            <li key={c.name} className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={isOn(c.name)}
                onChange={(e) => setSelected((s) => ({ ...s, [c.name]: e.target.checked }))}
              />
              <span className="min-w-0 flex-1 truncate">
                {c.name} - {c.username ? `${c.username}@` : ''}
                {c.host}:{c.port}
              </span>
            </li>
          ))}
        </ul>
      )}
    </PageShell>
  )
}
