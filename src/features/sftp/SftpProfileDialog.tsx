import { useEffect, useState } from 'react'
import type { AuthMethod, SftpProfile } from '@/bindings'
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
import { type KeyDraft, KeyListEditor, persistKeys } from '@/features/profiles/KeyListEditor'
import { profileSecretIds } from '@/lib/profiles'
import { deleteSecret, setSecret } from '@/lib/vault'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'

export function SftpProfileDialog({
  open,
  onOpenChange,
  editing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: SftpProfile | null
}) {
  const save = useSftpProfileStore((s) => s.save)
  const profiles = useSftpProfileStore((s) => s.profiles)
  const [name, setName] = useState('')
  const [host, setHost] = useState('')
  const [port, setPort] = useState(22)
  const [username, setUsername] = useState('')
  const [authMethod, setAuthMethod] = useState<AuthMethod>('password')
  const [password, setPassword] = useState('')
  const [keys, setKeys] = useState<KeyDraft[]>([])

  useEffect(() => {
    if (!open) return
    setName(editing?.name ?? '')
    setHost(editing?.host ?? '')
    setPort(editing?.port ?? 22)
    setUsername(editing?.username ?? '')
    setAuthMethod(editing?.authMethod ?? 'password')
    setPassword('')
    setKeys(
      (editing?.keys ?? []).map((k) => ({ path: k.path, secretId: k.secretId, passphrase: '' })),
    )
  }, [open, editing])

  const portValid = Number.isInteger(port) && port >= 1 && port <= 65535
  // Key auth with no usable key would save a profile that can never connect.
  const keysValid = authMethod !== 'key' || keys.some((k) => k.path.trim())

  const submit = async () => {
    const usesPassword = authMethod === 'password'
    let secretId = usesPassword ? (editing?.secretId ?? null) : null
    if (usesPassword && password) {
      secretId = await setSecret(password)
    }
    const nextKeys = authMethod === 'key' ? await persistKeys(keys) : []
    // Release vault entries this profile no longer points at.
    const kept = new Set([secretId, ...nextKeys.map((k) => k.secretId)].filter(Boolean))
    for (const stale of editing ? profileSecretIds(editing) : []) {
      if (!kept.has(stale)) await deleteSecret(stale).catch(() => {})
    }
    await save({
      id: editing?.id ?? crypto.randomUUID(),
      name: name || host,
      host,
      port,
      username,
      authMethod,
      keys: nextKeys,
      secretId,
      icon: editing?.icon ?? { kind: 'builtin', name: 'server' },
      order: editing?.order ?? profiles.length,
    })
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{editing ? 'Edit SFTP connection' : 'New SFTP connection'}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="sftp-profile-name">Name</Label>
            <Input
              id="sftp-profile-name"
              autoFocus
              value={name}
              placeholder={host || 'files.example.com'}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="sftp-profile-host">Host</Label>
            <Input id="sftp-profile-host" value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="sftp-profile-port">Port</Label>
            <Input
              id="sftp-profile-port"
              type="number"
              value={port}
              onChange={(e) => setPort(Number(e.target.value) || 22)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="sftp-profile-user">Username</Label>
            <Input
              id="sftp-profile-user"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="sftp-profile-auth">Auth method</Label>
            <Select value={authMethod} onValueChange={(v) => setAuthMethod(v as AuthMethod)}>
              <SelectTrigger id="sftp-profile-auth">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="password">Password</SelectItem>
                <SelectItem value="key">Private key</SelectItem>
                <SelectItem value="agent">SSH agent</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {authMethod === 'key' && <KeyListEditor keys={keys} onChange={setKeys} />}
          {authMethod === 'password' && (
            <div className="space-y-1">
              <Label htmlFor="sftp-profile-password">Password</Label>
              <Input
                id="sftp-profile-password"
                type="password"
                value={password}
                placeholder={editing?.secretId ? 'unchanged' : ''}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!host || !username || !portValid || !keysValid}
            onClick={submit}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
