import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { useEffect, useState } from 'react'
import type { AuthMethod } from '@/bindings'
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
import { ProfileIcon } from '@/features/profiles/ProfileIcon'
import { useProfileStore } from '@/stores/profileStore'

// What the form collects: the plaintext secret is exchanged for a vault id before it
// reaches the session store.
export interface SftpManualParams {
  host: string
  port: number
  username: string
  authMethod: AuthMethod
  keyPath: string
  secret: string
}

export function SftpConnectDialog({
  open,
  onOpenChange,
  onPick,
  onConnect,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (profileId: string, title: string) => void
  onConnect: (params: SftpManualParams) => void
}) {
  const profiles = useProfileStore((s) => s.profiles)
  const [query, setQuery] = useState('')
  const [manual, setManual] = useState(false)
  const [host, setHost] = useState('')
  const [port, setPort] = useState(22)
  const [username, setUsername] = useState('')
  const [authMethod, setAuthMethod] = useState<AuthMethod>('password')
  const [keyPath, setKeyPath] = useState('')
  const [secret, setSecret] = useState('')

  useEffect(() => {
    if (!open) return
    setManual(false)
    setHost('')
    setPort(22)
    setUsername('')
    setAuthMethod('password')
    setKeyPath('')
    setSecret('')
  }, [open])

  const portValid = Number.isInteger(port) && port >= 1 && port <= 65535
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
        {manual ? (
          <>
            <div className="space-y-2">
              <div className="space-y-1">
                <Label htmlFor="sftp-host">Host</Label>
                <Input
                  id="sftp-host"
                  autoFocus
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="sftp-port">Port</Label>
                <Input
                  id="sftp-port"
                  type="number"
                  value={port}
                  onChange={(e) => setPort(Number(e.target.value) || 22)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="sftp-username">Username</Label>
                <Input
                  id="sftp-username"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="sftp-auth">Auth method</Label>
                <Select value={authMethod} onValueChange={(v) => setAuthMethod(v as AuthMethod)}>
                  <SelectTrigger id="sftp-auth">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="password">Password</SelectItem>
                    <SelectItem value="key">Private key</SelectItem>
                    <SelectItem value="agent">SSH agent</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {authMethod === 'key' && (
                <div className="space-y-1">
                  <Label htmlFor="sftp-key-path">Key path</Label>
                  <div className="flex gap-2">
                    <Input
                      id="sftp-key-path"
                      value={keyPath}
                      onChange={(e) => setKeyPath(e.target.value)}
                    />
                    <Button
                      type="button"
                      variant="secondary"
                      onClick={async () => {
                        const picked = await pickFile({ multiple: false, directory: false })
                        if (typeof picked === 'string') setKeyPath(picked)
                      }}
                    >
                      Browse
                    </Button>
                  </div>
                </div>
              )}
              {authMethod !== 'agent' && (
                <div className="space-y-1">
                  <Label htmlFor="sftp-secret">
                    {authMethod === 'key' ? 'Passphrase' : 'Password'}
                  </Label>
                  <Input
                    id="sftp-secret"
                    type="password"
                    value={secret}
                    onChange={(e) => setSecret(e.target.value)}
                  />
                </div>
              )}
            </div>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setManual(false)}>
                Back
              </Button>
              <Button
                type="button"
                disabled={!host || !username || !portValid || (authMethod === 'key' && !keyPath)}
                onClick={() => {
                  onConnect({
                    host,
                    port,
                    username,
                    authMethod,
                    keyPath: authMethod === 'key' ? keyPath : '',
                    secret: authMethod === 'agent' ? '' : secret,
                  })
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
                    onClick={() => pick(p.id, p.name)}
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
