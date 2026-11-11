import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ProfileIcon } from '@/features/hosts/ProfileIcon'
import {
  type AdhocForm,
  type AdhocInitial,
  AdhocLoginFields,
  adhocForm,
  adhocReady,
} from '@/features/sessions/AdhocLoginFields'
import { connectionLabel, effectiveAddress } from '@/lib/hosts'
import { useHostStore } from '@/stores/hostStore'

// With `initial` (connect once from an address) the dialog opens on the manual form, filled in.
export function SftpConnectDialog({
  open,
  onOpenChange,
  onPick,
  onConnect,
  initial,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (profileId: string, title: string) => void
  onConnect: (params: AdhocForm) => void
  initial?: AdhocInitial
}) {
  const hosts = useHostStore((s) => s.hosts)
  const logins = useHostStore((s) => s.logins)
  const profiles = hosts
    .slice()
    .sort((a, b) => a.order - b.order)
    .flatMap((h) => {
      const ssh = h.connections.filter((c) => c.kind.type === 'ssh')
      return ssh.map((c) => ({
        id: c.id,
        title: h.name,
        name: ssh.length > 1 ? `${h.name} · ${connectionLabel(h, c, logins)}` : h.name,
        host: effectiveAddress(h, c),
        icon: h.icon,
      }))
    })
  const [query, setQuery] = useState('')
  const [manual, setManual] = useState(false)
  const [form, setForm] = useState<AdhocForm>(() => adhocForm(22))

  useEffect(() => {
    if (!open) return
    setManual(!!initial)
    setForm(adhocForm(22, initial))
  }, [open, initial])

  const q = query.trim().toLowerCase()
  const filtered = q
    ? profiles.filter((p) => p.name.toLowerCase().includes(q) || p.host.toLowerCase().includes(q))
    : profiles

  const pick = (id: string, name: string) => {
    onPick(id, name)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Open SFTP connection</DialogTitle>
        </DialogHeader>
        {manual ? (
          <>
            <AdhocLoginFields idPrefix="sftp" value={form} onChange={setForm} />
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setManual(false)}>
                Back
              </Button>
              <Button
                type="button"
                disabled={!adhocReady(form)}
                onClick={() => {
                  onConnect(form)
                  onOpenChange(false)
                }}
              >
                Connect
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <Input
              autoFocus
              placeholder="Search hosts..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <div className="max-h-72 space-y-0.5 overflow-y-auto">
              {filtered.length === 0 ? (
                <p className="px-1 py-6 text-center text-muted-foreground text-sm">
                  {profiles.length === 0 ? 'No saved hosts yet.' : 'No matching hosts.'}
                </p>
              ) : (
                filtered.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => pick(p.id, p.title)}
                    className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-muted"
                  >
                    <ProfileIcon icon={p.icon} className="size-4 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate">{p.name}</span>
                    <span className="shrink-0 truncate text-muted-foreground text-xs">
                      {p.host}
                    </span>
                  </button>
                ))
              )}
            </div>
            <Button type="button" variant="outline" onClick={() => setManual(true)}>
              Connect manually…
            </Button>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
