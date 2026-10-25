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
import { Label } from '@/components/ui/label'

export function RdpConnectDialog({
  open,
  onOpenChange,
  onConnect,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConnect: (
    host: string,
    port: number,
    username: string,
    domain: string | null,
    password: string,
  ) => void
}) {
  const [host, setHost] = useState('')
  const [port, setPort] = useState('3389')
  const [username, setUsername] = useState('')
  const [domain, setDomain] = useState('')
  const [password, setPassword] = useState('')

  // Cleared on close, so a password never stays behind in the hidden dialog. The other fields
  // are reset on open, so the dialog does not look wiped while it fades out.
  useEffect(() => {
    if (!open) {
      setPassword('')
      return
    }
    setHost('')
    setPort('3389')
    setUsername('')
    setDomain('')
  }, [open])

  const portNumber = Number(port)
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535
  const user = username.trim()
  const domainTrimmed = domain.trim()
  const domainTwice = domainTrimmed !== '' && (user.includes('\\') || user.includes('@'))

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New RDP connection</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="rdp-host">Host</Label>
            <Input id="rdp-host" autoFocus value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-port">Port</Label>
            <Input
              id="rdp-port"
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-username">Username</Label>
            <Input
              id="rdp-username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-domain">Domain (optional)</Label>
            <Input
              id="rdp-domain"
              value={domain}
              aria-invalid={domainTwice}
              aria-describedby={domainTwice ? 'rdp-domain-rule' : undefined}
              onChange={(e) => setDomain(e.target.value)}
            />
            {domainTwice && (
              <p id="rdp-domain-rule" className="text-destructive text-xs">
                Put the domain either in the user name or in the Domain field.
              </p>
            )}
          </div>
          <div className="space-y-1">
            <Label htmlFor="rdp-password">Password</Label>
            <Input
              id="rdp-password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!open || !host.trim() || !portValid || domainTwice}
            onClick={(e) => {
              // The closing dialog can still be clicked while it fades out.
              if (!open) return
              onConnect(host.trim(), portNumber, user, domainTrimmed || null, password)
              onOpenChange(false)
              // Otherwise the fading button keeps the focus the new tab's view looks for.
              e.currentTarget.blur()
            }}
          >
            Connect
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
