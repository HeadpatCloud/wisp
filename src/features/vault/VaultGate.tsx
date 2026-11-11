import { useEffect, useRef, useState } from 'react'
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
import { vaultCanSkipPassword, vaultSkipPassword, vaultStatus, vaultUnlock } from '@/lib/vault'

export function VaultGate({ onReady }: { onReady?: () => void } = {}) {
  const [needs, setNeeds] = useState(false)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [canSkip, setCanSkip] = useState(false)
  const readyRef = useRef(onReady)
  readyRef.current = onReady

  useEffect(() => {
    vaultStatus()
      .then((s) => {
        const locked = s === 'needsPassword'
        setNeeds(locked)
        if (!locked) readyRef.current?.()
        else vaultCanSkipPassword().then(setCanSkip, () => setCanSkip(false))
      })
      .catch(() => {
        // A status probe failure leaves the app usable, so the app-ready path must still run.
        setNeeds(false)
        readyRef.current?.()
      })
  }, [])

  async function submit() {
    if (!password || busy) return
    setBusy(true)
    setError(null)
    try {
      await vaultUnlock(password)
      setPassword('')
      setNeeds(false)
      readyRef.current?.()
    } catch (e) {
      const err = e as AppError
      setError(
        err?.kind === 'wrongPassphrase' ? 'Incorrect password.' : 'Could not unlock the vault.',
      )
    } finally {
      setBusy(false)
    }
  }

  async function skip() {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await vaultSkipPassword()
      setNeeds(false)
      readyRef.current?.()
    } catch {
      setError('Could not open the vault.')
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
        {canSkip && (
          <p className="text-muted-foreground text-sm">
            Wisp 0.5.0 stopped asking for this password after it was set. Without it, the passwords
            you saved since then still work and older ones stay locked, and Wisp won't ask again.
          </p>
        )}
        <DialogFooter>
          {canSkip && (
            <Button type="button" variant="ghost" disabled={busy} onClick={skip}>
              Continue without it
            </Button>
          )}
          <Button type="button" disabled={!password || busy} onClick={submit}>
            Unlock
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
