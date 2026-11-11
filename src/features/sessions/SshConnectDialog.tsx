import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  type AdhocForm,
  type AdhocInitial,
  AdhocLoginFields,
  adhocForm,
  adhocReady,
} from './AdhocLoginFields'

export function SshConnectDialog({
  open,
  onOpenChange,
  onConnect,
  initial,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConnect: (form: AdhocForm) => void
  initial?: AdhocInitial
}) {
  const [form, setForm] = useState<AdhocForm>(() => adhocForm(22, initial))

  useEffect(() => {
    if (open) setForm(adhocForm(22, initial))
  }, [open, initial])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Connect to SSH once</DialogTitle>
        </DialogHeader>
        <AdhocLoginFields idPrefix="ssh" value={form} onChange={setForm} />
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!adhocReady(form)}
            onClick={() => {
              onConnect(form)
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
