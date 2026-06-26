import { useEffect, useState } from 'react'
import { commands, type S3Profile } from '@/bindings'
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
import { unwrap } from '@/lib/ipc'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'

export function S3ProfileDialog({
  open,
  onOpenChange,
  editing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: S3Profile | null
}) {
  const save = useS3ProfileStore((s) => s.save)
  const [name, setName] = useState('')
  const [endpoint, setEndpoint] = useState('')
  const [port, setPort] = useState('')
  const [region, setRegion] = useState('us-east-1')
  const [useTls, setUseTls] = useState(true)
  const [pathStyle, setPathStyle] = useState(true)
  const [accessKeyId, setAccessKeyId] = useState('')
  const [secretKey, setSecretKey] = useState('')
  const [bucket, setBucket] = useState('')

  useEffect(() => {
    if (!open) return
    setName(editing?.name ?? '')
    setEndpoint(editing?.endpoint ?? '')
    setPort(editing?.port != null ? String(editing.port) : '')
    setRegion(editing?.region ?? 'us-east-1')
    setUseTls(editing?.useTls ?? true)
    setPathStyle(editing?.pathStyle ?? true)
    setAccessKeyId(editing?.accessKeyId ?? '')
    setSecretKey('')
    setBucket(editing?.bucket ?? '')
  }, [open, editing])

  const submit = async () => {
    let secretId = editing?.secretId ?? null
    if (secretKey) {
      const newId = unwrap(await commands.setSecret(secretKey))
      if (editing?.secretId) await commands.deleteSecret(editing.secretId)
      secretId = newId
    }
    const profile: S3Profile = {
      id: editing?.id ?? crypto.randomUUID(),
      name: name || endpoint,
      endpoint,
      port: port ? Number(port) : null,
      region,
      useTls,
      pathStyle,
      accessKeyId,
      secretId,
      bucket: bucket || null,
      icon: editing?.icon ?? { kind: 'builtin', name: 'server' },
      order: editing?.order ?? 0,
    }
    await save(profile)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{editing ? 'Edit S3 connection' : 'New S3 connection'}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="s3-name">Name</Label>
            <Input id="s3-name" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="grid grid-cols-[1fr_6rem] gap-2">
            <div className="space-y-1">
              <Label htmlFor="s3-endpoint">Endpoint host</Label>
              <Input
                id="s3-endpoint"
                placeholder="s3.amazonaws.com"
                value={endpoint}
                onChange={(e) => setEndpoint(e.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="s3-port">Port</Label>
              <Input
                id="s3-port"
                type="number"
                placeholder="auto"
                value={port}
                onChange={(e) => setPort(e.target.value)}
              />
            </div>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-1">
              <Label htmlFor="s3-region">Region</Label>
              <Input id="s3-region" value={region} onChange={(e) => setRegion(e.target.value)} />
            </div>
            <div className="space-y-1">
              <Label htmlFor="s3-bucket">Bucket (optional)</Label>
              <Input id="s3-bucket" value={bucket} onChange={(e) => setBucket(e.target.value)} />
            </div>
          </div>
          <div className="space-y-1">
            <Label htmlFor="s3-access">Access key ID</Label>
            <Input
              id="s3-access"
              value={accessKeyId}
              onChange={(e) => setAccessKeyId(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="s3-secret">Secret access key</Label>
            <Input
              id="s3-secret"
              type="password"
              placeholder={editing?.secretId ? 'unchanged' : ''}
              value={secretKey}
              onChange={(e) => setSecretKey(e.target.value)}
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={useTls} onChange={(e) => setUseTls(e.target.checked)} />
            Use TLS (https)
          </label>
          {!useTls && (
            <p className="pl-6 text-destructive text-xs">
              Without TLS your keys and data are sent unencrypted.
            </p>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={pathStyle}
              onChange={(e) => setPathStyle(e.target.checked)}
            />
            Path-style addressing (needed for most S3-compatible servers)
          </label>
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" disabled={!endpoint || !accessKeyId} onClick={submit}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
