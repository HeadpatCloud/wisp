import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { ChevronDown, ChevronUp, Plus, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { deleteSecret, setSecret } from '@/lib/vault'

// A key being edited: `passphrase` is plaintext until save swaps it for a vault id, and
// `secretId` is the one already stored (kept so an untouched key isn't re-saved).
export interface KeyDraft {
  path: string
  secretId: string | null
  passphrase: string
}

export function KeyListEditor({
  keys,
  onChange,
}: {
  keys: KeyDraft[]
  onChange: (keys: KeyDraft[]) => void
}) {
  const update = (i: number, patch: Partial<KeyDraft>) =>
    onChange(keys.map((k, at) => (at === i ? { ...k, ...patch } : k)))

  const move = (i: number, delta: number) => {
    const to = i + delta
    if (to < 0 || to >= keys.length) return
    const next = [...keys]
    ;[next[i], next[to]] = [next[to], next[i]]
    onChange(next)
  }

  return (
    <div className="space-y-2">
      <Label>Private keys</Label>
      {keys.length === 0 ? (
        <p className="text-muted-foreground text-xs">
          No keys yet. Add one, or use SSH agent instead.
        </p>
      ) : (
        <p className="text-muted-foreground text-xs">Tried top to bottom until one is accepted.</p>
      )}
      {keys.map((k, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: rows are positional and reorderable
        <div key={i} className="space-y-1 rounded border border-border p-2">
          <div className="flex gap-2">
            <Input
              value={k.path}
              placeholder="/home/me/.ssh/id_ed25519"
              onChange={(e) => update(i, { path: e.target.value })}
            />
            <Button
              type="button"
              variant="secondary"
              onClick={async () => {
                const picked = await pickFile({ multiple: false, directory: false })
                if (typeof picked === 'string') update(i, { path: picked })
              }}
            >
              Browse
            </Button>
          </div>
          <div className="flex gap-2">
            <Input
              type="password"
              value={k.passphrase}
              placeholder={k.secretId ? 'passphrase unchanged' : 'passphrase (optional)'}
              onChange={(e) => update(i, { passphrase: e.target.value })}
            />
            <button
              type="button"
              aria-label={`Move key ${i + 1} up`}
              disabled={i === 0}
              onClick={() => move(i, -1)}
              className="rounded p-1 hover:bg-muted disabled:opacity-40"
            >
              <ChevronUp className="size-4" />
            </button>
            <button
              type="button"
              aria-label={`Move key ${i + 1} down`}
              disabled={i === keys.length - 1}
              onClick={() => move(i, 1)}
              className="rounded p-1 hover:bg-muted disabled:opacity-40"
            >
              <ChevronDown className="size-4" />
            </button>
            <button
              type="button"
              aria-label={`Remove key ${i + 1}`}
              onClick={() => onChange(keys.filter((_, at) => at !== i))}
              className="rounded p-1 text-destructive hover:bg-muted"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>
      ))}
      <Button
        type="button"
        variant="outline"
        onClick={() => onChange([...keys, { path: '', secretId: null, passphrase: '' }])}
      >
        <Plus className="size-4" />
        Add key
      </Button>
    </div>
  )
}

// Swaps each freshly typed passphrase for a vault id, leaving untouched keys alone.
export async function persistKeys(
  keys: KeyDraft[],
): Promise<{ path: string; secretId: string | null }[]> {
  const out: { path: string; secretId: string | null }[] = []
  for (const k of keys) {
    if (!k.path.trim()) continue
    let secretId = k.secretId
    if (k.passphrase) {
      secretId = await setSecret(k.passphrase)
      if (k.secretId) await deleteSecret(k.secretId).catch(() => {})
    }
    out.push({ path: k.path.trim(), secretId })
  }
  return out
}
