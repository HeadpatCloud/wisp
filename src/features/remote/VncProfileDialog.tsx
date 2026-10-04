import { useEffect, useState } from 'react'
import type { VncProfile } from '@/bindings'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { deleteSecret, setSecret } from '@/lib/vault'
import { useVncProfileStore } from '@/stores/vncProfileStore'

export function VncProfileDialog({
  open,
  onOpenChange,
  editing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: VncProfile | null
}) {
  const save = useVncProfileStore((s) => s.save)
  const [name, setName] = useState('')
  const [host, setHost] = useState('')
  const [port, setPort] = useState('5900')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [removePassword, setRemovePassword] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // Filled on open and emptied on close, so a password never stays behind in the hidden dialog.
  useEffect(() => {
    const shown = open ? editing : null
    setName(shown?.name ?? '')
    setHost(shown?.host ?? '')
    setPort(String(shown?.port ?? 5900))
    setUsername(shown?.username ?? '')
    setPassword('')
    setRemovePassword(false)
    setError(null)
  }, [open, editing])

  const target = host.trim()
  const portNumber = Number(port)
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535
  const savedSecret = editing?.secretId ?? null

  const submit = async () => {
    setError(null)
    // Save is disabled meanwhile: a second run would store another secret and another profile.
    setSaving(true)
    let created: string | null = null
    try {
      if (password) created = await setSecret(password)
      await save({
        id: editing?.id ?? crypto.randomUUID(),
        name: name.trim() || `${target}:${portNumber}`,
        host: target,
        port: portNumber,
        username: username.trim() || null,
        secretId: removePassword ? null : (created ?? savedSecret),
        icon: editing?.icon ?? { kind: 'builtin', name: 'server' },
        order: editing?.order ?? 0,
      })
    } catch (e) {
      // The profile still has its old password, so only the entry made for this save goes.
      if (created) await deleteSecret(created).catch(() => {})
      setError(e instanceof Error ? e.message : String(e))
      setSaving(false)
      return
    }
    // Not before the save: a save that fails must leave the profile its password. Best effort,
    // because the profile no longer points at this entry whether or not it could be deleted.
    if (savedSecret && (created || removePassword)) await deleteSecret(savedSecret).catch(() => {})
    setSaving(false)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{editing ? 'Edit VNC profile' : 'New VNC profile'}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="vnc-profile-name">Name</Label>
            <Input
              id="vnc-profile-name"
              autoFocus
              value={name}
              placeholder={target ? `${target}:${port}` : ''}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-profile-host">Host</Label>
            <Input id="vnc-profile-host" value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-profile-port">Port</Label>
            <Input
              id="vnc-profile-port"
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-profile-username">Username (optional)</Label>
            <Input
              id="vnc-profile-username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
            <p className="text-muted-foreground text-xs">
              Only for macOS Screen Sharing and servers that ask for one.
            </p>
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-profile-password">Password</Label>
            <Input
              id="vnc-profile-password"
              type="password"
              value={password}
              disabled={removePassword}
              placeholder={savedSecret && !removePassword ? 'unchanged' : ''}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          {savedSecret && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={removePassword}
                onChange={(e) => {
                  setRemovePassword(e.target.checked)
                  if (e.target.checked) setPassword('')
                }}
              />
              Remove saved password
            </label>
          )}
          {error && <p className="text-destructive text-xs">{error}</p>}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" disabled={!target || !portValid || saving} onClick={submit}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
