import { useEffect, useState } from 'react'
import type { RdpProfile } from '@/bindings'
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { deleteSecret, setSecret } from '@/lib/vault'
import { useRdpProfileStore } from '@/stores/rdpProfileStore'

const AUTOMATIC = 'automatic'

const LAYOUTS: [number, string][] = [
  [0x409, 'English (US)'],
  [0x20409, 'English (US International)'],
  [0x809, 'English (UK)'],
  [0x407, 'German'],
  [0x807, 'German (Switzerland)'],
  [0x40c, 'French'],
  [0x80c, 'French (Belgium)'],
  [0x100c, 'French (Switzerland)'],
  [0x413, 'Dutch'],
  [0x813, 'Belgian (Period)'],
  [0x40a, 'Spanish'],
  [0x410, 'Italian'],
  [0x816, 'Portuguese'],
  [0x416, 'Portuguese (Brazil)'],
  [0x406, 'Danish'],
  [0x414, 'Norwegian'],
  [0x41d, 'Swedish'],
  [0x40b, 'Finnish'],
  [0x415, 'Polish (Programmers)'],
  [0x405, 'Czech'],
  [0x40e, 'Hungarian'],
  [0x41f, 'Turkish Q'],
  [0x419, 'Russian'],
  [0x411, 'Japanese'],
]

export function RdpProfileDialog({
  open,
  onOpenChange,
  editing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: RdpProfile | null
}) {
  const save = useRdpProfileStore((s) => s.save)
  const [name, setName] = useState('')
  const [host, setHost] = useState('')
  const [port, setPort] = useState('3389')
  const [username, setUsername] = useState('')
  const [domain, setDomain] = useState('')
  const [layout, setLayout] = useState(AUTOMATIC)
  const [password, setPassword] = useState('')
  const [removePassword, setRemovePassword] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // Filled on open and emptied on close, so a password never stays behind in the hidden dialog.
  // The other fields keep their text on close, so the dialog does not look wiped while it fades
  // out.
  useEffect(() => {
    setPassword('')
    setRemovePassword(false)
    setError(null)
    if (!open) return
    setName(editing?.name ?? '')
    setHost(editing?.host ?? '')
    setPort(String(editing?.port ?? 3389))
    setUsername(editing?.username ?? '')
    setDomain(editing?.domain ?? '')
    setLayout(editing?.keyboardLayout == null ? AUTOMATIC : String(editing.keyboardLayout))
  }, [open, editing])

  const target = host.trim()
  const portNumber = Number(port)
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535
  const user = username.trim()
  const domainTrimmed = domain.trim()
  const domainTwice = domainTrimmed !== '' && (user.includes('\\') || user.includes('@'))
  const savedSecret = editing?.secretId ?? null
  const stored = editing?.keyboardLayout ?? null
  const unlisted = stored !== null && !LAYOUTS.some(([id]) => id === stored) ? stored : null

  const submit = async () => {
    // The closing dialog can still be clicked while it fades out.
    if (!open) return
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
        username: user,
        domain: domainTrimmed || null,
        secretId: removePassword ? null : (created ?? savedSecret),
        keyboardLayout: layout === AUTOMATIC ? null : Number(layout),
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
          <DialogTitle>{editing ? 'Edit RDP profile' : 'New RDP profile'}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-name">Name</Label>
            <Input
              id="rdp-profile-name"
              autoFocus
              value={name}
              placeholder={target ? `${target}:${port}` : ''}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-host">Host</Label>
            <Input id="rdp-profile-host" value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-port">Port</Label>
            <Input
              id="rdp-profile-port"
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-username">Username</Label>
            <Input
              id="rdp-profile-username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-domain">Domain (optional)</Label>
            <Input
              id="rdp-profile-domain"
              value={domain}
              aria-invalid={domainTwice}
              aria-describedby={domainTwice ? 'rdp-profile-domain-rule' : undefined}
              onChange={(e) => setDomain(e.target.value)}
            />
            {domainTwice && (
              <p id="rdp-profile-domain-rule" className="text-destructive text-xs">
                Put the domain either in the user name or in the Domain field.
              </p>
            )}
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-password">Password</Label>
            <Input
              id="rdp-profile-password"
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
          <div className="space-y-1">
            <Label htmlFor="rdp-profile-layout">Keyboard layout</Label>
            <Select value={layout} onValueChange={setLayout}>
              <SelectTrigger id="rdp-profile-layout">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={AUTOMATIC}>Automatic</SelectItem>
                {LAYOUTS.map(([id, label]) => (
                  <SelectItem key={id} value={String(id)}>
                    {label}
                  </SelectItem>
                ))}
                {unlisted !== null && (
                  <SelectItem value={String(unlisted)}>{`0x${unlisted.toString(16)}`}</SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>
          {error && (
            <p role="alert" className="text-destructive text-xs">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!open || !target || !portValid || domainTwice || saving}
            onClick={submit}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
