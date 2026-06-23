import { useState } from 'react'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ProfileIcon } from '@/features/profiles/ProfileIcon'
import { useProfileStore } from '@/stores/profileStore'

export function SftpConnectDialog({
  open,
  onOpenChange,
  onPick,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (profileId: string, title: string) => void
}) {
  const profiles = useProfileStore((s) => s.profiles)
  const [query, setQuery] = useState('')
  const q = query.trim().toLowerCase()
  const filtered = (
    q
      ? profiles.filter((p) => p.name.toLowerCase().includes(q) || p.host.toLowerCase().includes(q))
      : profiles
  )
    .slice()
    .sort((a, b) => a.order - b.order)

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
                onClick={() => pick(p.id, p.name)}
                className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-muted"
              >
                <ProfileIcon icon={p.icon} className="size-4 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{p.name}</span>
                <span className="shrink-0 truncate text-muted-foreground text-xs">{p.host}</span>
              </button>
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
