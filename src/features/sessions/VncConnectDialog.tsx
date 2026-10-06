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

export function VncConnectDialog({
  open,
  onOpenChange,
  onConnect,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConnect: (host: string, port: number, username: string, password: string) => void
}) {
  const [host, setHost] = useState('')
  const [port, setPort] = useState('5900')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')

  // Cleared on close, so a password never stays behind in the hidden dialog. The other fields
  // are reset on open, so the dialog does not look wiped while it fades out.
  useEffect(() => {
    if (!open) {
      setPassword('')
      return
    }
    setHost('')
    setPort('5900')
    setUsername('')
  }, [open])

  const portNumber = Number(port)
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New VNC connection</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="vnc-host">Host</Label>
            <Input id="vnc-host" autoFocus value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-port">Port</Label>
            <Input
              id="vnc-port"
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-username">Username (optional)</Label>
            <Input
              id="vnc-username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
            <p className="text-muted-foreground text-xs">
              Only for macOS Screen Sharing and servers that ask for one.
            </p>
          </div>
          <div className="space-y-1">
            <Label htmlFor="vnc-password">Password</Label>
            <Input
              id="vnc-password"
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
            disabled={!open || !host || !portValid}
            onClick={(e) => {
              // The closing dialog can still be clicked while it fades out.
              if (!open) return
              onConnect(host, portNumber, username, password)
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
