import { X } from 'lucide-react'
import type { TunnelKind } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { TunnelDraft } from './drafts'
import { Check } from './fields'

const LOOPBACK = ['127.0.0.1', 'localhost', '::1']

export function TunnelRows({
  tunnels,
  onChange,
  errors,
}: {
  tunnels: TunnelDraft[]
  onChange: (t: TunnelDraft[]) => void
  errors: Record<string, string>
}) {
  const set = (id: string, patch: Partial<TunnelDraft>) =>
    onChange(tunnels.map((t) => (t.id === id ? { ...t, ...patch } : t)))

  return (
    <fieldset className="col-span-full flex flex-col gap-2">
      <legend className="mb-2 font-medium text-sm">Tunnels</legend>
      {tunnels.map((t) => {
        const exposed = t.bindHost.trim() !== '' && !LOOPBACK.includes(t.bindHost.trim())
        const portError = errors[`tunnel-${t.id}-bindPort`] ?? errors[`tunnel-${t.id}-targetPort`]
        return (
          <div key={t.id} className="flex flex-col gap-2 rounded-lg border border-border p-2">
            <div className="flex flex-wrap items-center gap-2">
              <select
                aria-label="Kind"
                value={t.kind}
                onChange={(e) => set(t.id, { kind: e.target.value as TunnelKind })}
                className="h-9 rounded-md border border-input bg-transparent px-2 text-sm"
              >
                <option value="local">Local</option>
                <option value="remote">Remote</option>
                <option value="dynamic">Dynamic</option>
              </select>
              <Input
                aria-label="Bind host"
                className="w-32"
                value={t.bindHost}
                onChange={(e) => set(t.id, { bindHost: e.target.value })}
              />
              <Input
                aria-label="Bind port"
                className="w-20"
                value={t.bindPort}
                onChange={(e) => set(t.id, { bindPort: e.target.value })}
              />
              {t.kind !== 'dynamic' && (
                <>
                  <span className="text-muted-foreground text-sm">→</span>
                  <Input
                    aria-label="Target host"
                    className="w-32"
                    value={t.targetHost}
                    onChange={(e) => set(t.id, { targetHost: e.target.value })}
                  />
                  <Input
                    aria-label="Target port"
                    className="w-20"
                    value={t.targetPort}
                    onChange={(e) => set(t.id, { targetPort: e.target.value })}
                  />
                </>
              )}
              <button
                type="button"
                aria-label="Remove tunnel"
                onClick={() => onChange(tunnels.filter((x) => x.id !== t.id))}
                className="ml-auto rounded p-1 text-muted-foreground hover:bg-muted"
              >
                <X className="size-4" aria-hidden />
              </button>
            </div>
            <Check
              label="Start when SSH connects"
              checked={t.autoStart}
              onChange={(autoStart) => set(t.id, { autoStart })}
            />
            {portError && <p className="text-destructive text-xs">{portError}</p>}
            {exposed && (
              <p className="text-destructive text-xs">
                Binding to {t.bindHost.trim()} exposes this forward beyond localhost to the network.
              </p>
            )}
          </div>
        )
      })}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="self-start"
        onClick={() =>
          onChange([
            ...tunnels,
            {
              id: crypto.randomUUID(),
              kind: 'local',
              bindHost: '127.0.0.1',
              bindPort: '8080',
              targetHost: '',
              targetPort: '',
              autoStart: false,
            },
          ])
        }
      >
        Add a tunnel
      </Button>
    </fieldset>
  )
}
