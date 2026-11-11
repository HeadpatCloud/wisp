import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { useConfirmStore } from '@/stores/confirmStore'

export function ConfirmDialog() {
  const request = useConfirmStore((s) => s.request)
  const answer = useConfirmStore((s) => s.answer)
  return (
    <Dialog open={request !== null} onOpenChange={(open) => !open && answer(false)}>
      {request && (
        <DialogContent showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>{request.title}</DialogTitle>
            <DialogDescription className="whitespace-pre-line">{request.body}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => answer(false)}>
              Cancel
            </Button>
            <Button
              variant={request.destructive ? 'destructive' : 'default'}
              onClick={() => answer(true)}
            >
              {request.confirmLabel}
            </Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}
