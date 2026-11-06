import { open as pickFolder } from '@tauri-apps/plugin-dialog'
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
  const [audio, setAudio] = useState(true)
  const [shareOn, setShareOn] = useState(false)
  const [folder, setFolder] = useState('')
  const [shareName, setShareName] = useState('')
  const [gatewayOn, setGatewayOn] = useState(false)
  const [gatewayHost, setGatewayHost] = useState('')
  const [gatewayPort, setGatewayPort] = useState('443')
  const [sameLogin, setSameLogin] = useState(true)
  const [gatewayUsername, setGatewayUsername] = useState('')
  const [gatewayPassword, setGatewayPassword] = useState('')
  const [removeGatewayPassword, setRemoveGatewayPassword] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // Filled on open and emptied on close, so a password never stays behind in the hidden dialog.
  // The other fields keep their text on close, so the dialog does not look wiped while it fades
  // out.
  useEffect(() => {
    setPassword('')
    setRemovePassword(false)
    setGatewayPassword('')
    setRemoveGatewayPassword(false)
    setError(null)
    if (!open) return
    setName(editing?.name ?? '')
    setHost(editing?.host ?? '')
    setPort(String(editing?.port ?? 3389))
    setUsername(editing?.username ?? '')
    setDomain(editing?.domain ?? '')
    setLayout(editing?.keyboardLayout == null ? AUTOMATIC : String(editing.keyboardLayout))
    setAudio(editing?.audio ?? true)
    setShareOn(editing?.sharedFolder != null)
    setFolder(editing?.sharedFolder?.path ?? '')
    setShareName(editing?.sharedFolder?.name ?? '')
    setGatewayOn(editing?.gateway != null)
    setGatewayHost(editing?.gateway?.host ?? '')
    setGatewayPort(String(editing?.gateway?.port ?? 443))
    setSameLogin(editing?.gateway?.sameLogin ?? true)
    setGatewayUsername(editing?.gateway?.username ?? '')
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
  const share = shareName.trim()
  const shareValid = /^[A-Za-z0-9_-]{1,7}$/.test(share)
  const shareInvalid = share !== '' && !shareValid
  const shareIncomplete = shareOn && (!folder || !shareValid)
  const gatewayTarget = gatewayHost.trim()
  const gatewayPortNumber = Number(gatewayPort)
  const gatewayPortValid =
    Number.isInteger(gatewayPortNumber) && gatewayPortNumber >= 1 && gatewayPortNumber <= 65535
  const ownLogin = gatewayOn && !sameLogin
  const gatewayUser = gatewayUsername.trim()
  const gatewayIncomplete =
    gatewayOn && (!gatewayTarget || !gatewayPortValid || (ownLogin && !gatewayUser))
  const savedGatewaySecret = editing?.gateway?.secretId ?? null

  const submit = async () => {
    // The closing dialog can still be clicked while it fades out.
    if (!open) return
    setError(null)
    // Save is disabled meanwhile: a second run would store another secret and another profile.
    setSaving(true)
    let created: string | null = null
    let createdGateway: string | null = null
    try {
      if (password) created = await setSecret(password)
      if (ownLogin && gatewayPassword) createdGateway = await setSecret(gatewayPassword)
      await save({
        id: editing?.id ?? crypto.randomUUID(),
        name: name.trim() || `${target}:${portNumber}`,
        host: target,
        port: portNumber,
        username: user,
        domain: domainTrimmed || null,
        secretId: removePassword ? null : (created ?? savedSecret),
        keyboardLayout: layout === AUTOMATIC ? null : Number(layout),
        audio,
        sharedFolder: shareOn ? { path: folder, name: share } : null,
        gateway: gatewayOn
          ? {
              host: gatewayTarget,
              port: gatewayPortNumber,
              sameLogin,
              username: ownLogin ? gatewayUser : '',
              secretId:
                ownLogin && !removeGatewayPassword ? (createdGateway ?? savedGatewaySecret) : null,
            }
          : null,
        icon: editing?.icon ?? { kind: 'builtin', name: 'server' },
        order: editing?.order ?? 0,
      })
    } catch (e) {
      // The profile still has its old passwords, so only the entries made for this save go.
      if (created) await deleteSecret(created).catch(() => {})
      if (createdGateway) await deleteSecret(createdGateway).catch(() => {})
      setError(e instanceof Error ? e.message : String(e))
      setSaving(false)
      return
    }
    // Not before the save: a save that fails must leave the profile its password. Best effort,
    // because the profile no longer points at this entry whether or not it could be deleted.
    if (savedSecret && (created || removePassword)) await deleteSecret(savedSecret).catch(() => {})
    if (savedGatewaySecret && (createdGateway || removeGatewayPassword || !ownLogin)) {
      await deleteSecret(savedGatewaySecret).catch(() => {})
    }
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
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={audio} onChange={(e) => setAudio(e.target.checked)} />
            Play sound from the remote computer
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={shareOn}
              onChange={(e) => setShareOn(e.target.checked)}
            />
            Share a folder
          </label>
          {shareOn && (
            <>
              <div className="space-y-1">
                <Label htmlFor="rdp-profile-folder">Folder</Label>
                <div className="flex gap-2">
                  <Input id="rdp-profile-folder" readOnly value={folder} />
                  <Button
                    type="button"
                    variant="secondary"
                    onClick={async () => {
                      const picked = await pickFolder({ directory: true })
                      if (typeof picked !== 'string') return
                      setFolder(picked)
                      if (share) return
                      const last = picked.split(/[\\/]/).filter(Boolean).at(-1) ?? ''
                      setShareName(last.replace(/[^A-Za-z0-9_-]/g, '').slice(0, 7) || 'share')
                    }}
                  >
                    Browse…
                  </Button>
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="rdp-profile-share-name">Share name</Label>
                <Input
                  id="rdp-profile-share-name"
                  value={shareName}
                  aria-invalid={shareInvalid}
                  aria-describedby={
                    shareInvalid
                      ? 'rdp-profile-share-hint rdp-profile-share-rule'
                      : 'rdp-profile-share-hint'
                  }
                  onChange={(e) => setShareName(e.target.value)}
                />
                <p id="rdp-profile-share-hint" className="text-muted-foreground text-xs">
                  {`Shows up as \\\\tsclient\\${share}`}
                </p>
                {shareInvalid && (
                  <p id="rdp-profile-share-rule" className="text-destructive text-xs">
                    A share name has 1 to 7 letters, digits, - or _.
                  </p>
                )}
              </div>
            </>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={gatewayOn}
              onChange={(e) => setGatewayOn(e.target.checked)}
            />
            Connect through an RD Gateway
          </label>
          {gatewayOn && (
            <>
              <div className="space-y-1">
                <Label htmlFor="rdp-profile-gateway-host">Gateway host</Label>
                <Input
                  id="rdp-profile-gateway-host"
                  value={gatewayHost}
                  onChange={(e) => setGatewayHost(e.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="rdp-profile-gateway-port">Gateway port</Label>
                <Input
                  id="rdp-profile-gateway-port"
                  type="number"
                  value={gatewayPort}
                  onChange={(e) => setGatewayPort(e.target.value)}
                />
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={sameLogin}
                  onChange={(e) => setSameLogin(e.target.checked)}
                />
                Use the same login for the gateway
              </label>
              {!sameLogin && (
                <>
                  <div className="space-y-1">
                    <Label htmlFor="rdp-profile-gateway-username">Gateway username</Label>
                    <Input
                      id="rdp-profile-gateway-username"
                      value={gatewayUsername}
                      onChange={(e) => setGatewayUsername(e.target.value)}
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="rdp-profile-gateway-password">Gateway password</Label>
                    <Input
                      id="rdp-profile-gateway-password"
                      type="password"
                      value={gatewayPassword}
                      disabled={removeGatewayPassword}
                      placeholder={savedGatewaySecret && !removeGatewayPassword ? 'unchanged' : ''}
                      onChange={(e) => setGatewayPassword(e.target.value)}
                    />
                  </div>
                  {savedGatewaySecret && (
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={removeGatewayPassword}
                        onChange={(e) => {
                          setRemoveGatewayPassword(e.target.checked)
                          if (e.target.checked) setGatewayPassword('')
                        }}
                      />
                      Remove saved gateway password
                    </label>
                  )}
                </>
              )}
            </>
          )}
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
            disabled={
              !open ||
              !target ||
              !portValid ||
              domainTwice ||
              shareIncomplete ||
              gatewayIncomplete ||
              saving
            }
            onClick={submit}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
