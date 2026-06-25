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

export interface FtpParams {
  host: string
  port: number
  username: string
  password: string
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
}

export function FtpConnectDialog({
  open,
  onOpenChange,
  onConnect,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConnect: (params: FtpParams) => void
}) {
  const [host, setHost] = useState('')
  const [port, setPort] = useState(21)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [secure, setSecure] = useState(true)
  const [allowInvalidCert, setAllowInvalidCert] = useState(false)
  const [ignoreHostname, setIgnoreHostname] = useState(false)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New FTP connection</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="ftp-host">Host</Label>
            <Input id="ftp-host" autoFocus value={host} onChange={(e) => setHost(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="ftp-port">Port</Label>
            <Input
              id="ftp-port"
              type="number"
              value={port}
              onChange={(e) => setPort(Number(e.target.value) || 21)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="ftp-username">Username</Label>
            <Input
              id="ftp-username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="ftp-password">Password</Label>
            <Input
              id="ftp-password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={secure} onChange={(e) => setSecure(e.target.checked)} />
            Use FTPS (explicit TLS)
          </label>
          {secure ? (
            <>
              <label className="flex items-center gap-2 pl-6 text-muted-foreground text-sm">
                <input
                  type="checkbox"
                  checked={allowInvalidCert}
                  onChange={(e) => setAllowInvalidCert(e.target.checked)}
                />
                Allow self-signed certificate
              </label>
              {allowInvalidCert && (
                <label className="flex items-center gap-2 pl-12 text-muted-foreground text-sm">
                  <input
                    type="checkbox"
                    checked={ignoreHostname}
                    onChange={(e) => setIgnoreHostname(e.target.checked)}
                  />
                  Also ignore hostname mismatch (less safe)
                </label>
              )}
            </>
          ) : (
            <p className="pl-6 text-destructive text-xs">
              Without FTPS your username, password, and files are sent unencrypted.
            </p>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!host}
            onClick={() => {
              onConnect({
                host,
                port,
                username,
                password,
                secure,
                allowInvalidCert,
                ignoreHostname,
              })
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
