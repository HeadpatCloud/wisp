import { useEffect, useState } from 'react'
import type { AppError } from '@/bindings'
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
import { vaultStatus, vaultUnlock } from '@/lib/vault'

export function VaultGate() {
  const [needs, setNeeds] = useState(false)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    vaultStatus()
      .then((s) => setNeeds(s === 'needsPassword'))
      .catch(() => setNeeds(false))
  }, [])

  async function submit() {
    if (!password || busy) return
    setBusy(true)
    setError(null)
    try {
      await vaultUnlock(password)
      setPassword('')
      setNeeds(false)
    } catch (e) {
      const err = e as AppError
      setError(
        err?.kind === 'wrongPassphrase' ? 'Incorrect password.' : 'Could not unlock the vault.',
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={needs}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Unlock vault</DialogTitle>
        </DialogHeader>
        <div className="space-y-1">
          <Label htmlFor="master-password">Master password</Label>
          <Input
            id="master-password"
            type="password"
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit()
            }}
          />
          {error && <p className="text-destructive text-xs">{error}</p>}
        </div>
        <DialogFooter>
          <Button type="button" disabled={!password || busy} onClick={submit}>
            Unlock
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
