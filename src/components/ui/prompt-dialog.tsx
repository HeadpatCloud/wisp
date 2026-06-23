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

export function PromptDialog({
  open,
  title,
  defaultValue = '',
  confirmLabel = 'OK',
  onConfirm,
  onOpenChange,
}: {
  open: boolean
  title: string
  defaultValue?: string
  confirmLabel?: string
  onConfirm: (value: string) => void
  onOpenChange: (open: boolean) => void
}) {
  const [value, setValue] = useState(defaultValue)
  useEffect(() => {
    if (open) setValue(defaultValue)
  }, [open, defaultValue])
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <Input
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && value.trim()) onConfirm(value.trim())
          }}
        />
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" disabled={!value.trim()} onClick={() => onConfirm(value.trim())}>
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
