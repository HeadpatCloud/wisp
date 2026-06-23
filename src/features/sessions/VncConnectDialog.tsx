import { useState } from 'react'
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
  onConnect: (host: string, port: number, password: string) => void
}) {
  const [host, setHost] = useState('')
  const [port, setPort] = useState(5900)
  const [password, setPassword] = useState('')

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
              onChange={(e) => setPort(Number(e.target.value) || 5900)}
            />
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
            disabled={!host}
            onClick={() => {
              onConnect(host, port, password)
              onOpenChange(false)
            }}
          >
            Connect
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
