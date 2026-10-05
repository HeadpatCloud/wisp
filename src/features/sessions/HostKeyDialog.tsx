import { useRef } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

export type HostKeyPrompt =
  | { kind: 'unknown'; host: string; port: number; fingerprint: string; certificate?: boolean }
  | {
      kind: 'mismatch'
      host: string
      port: number
      stored: string
      offered: string
      certificate?: boolean
    }

export function HostKeyDialog({
  prompt,
  onAccept,
  onReject,
}: {
  prompt: HostKeyPrompt | null
  onAccept: () => void
  onReject: () => void
}) {
  // The closing dialog still shows the prompt it was opened for.
  const lastRef = useRef(prompt)
  if (prompt) lastRef.current = prompt
  const shown = prompt ?? lastRef.current

  return (
    <Dialog open={prompt !== null} onOpenChange={(open) => !open && onReject()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {shown?.kind === 'mismatch'
              ? shown.certificate
                ? 'Certificate CHANGED'
                : 'Host key CHANGED'
              : shown?.certificate
                ? 'Unknown certificate'
                : 'Unknown host key'}
          </DialogTitle>
        </DialogHeader>
        {shown && (
          <div className="space-y-2 text-sm">
            <p className="text-muted-foreground">
              {shown.certificate ? shown.host.replace(/^(vnc|rdp|rdg)\//, '') : shown.host}:
              {shown.port}
            </p>
            {shown.kind === 'unknown' ? (
              <>
                <p>
                  {shown.certificate
                    ? 'First connection to this server. Trust this certificate?'
                    : 'First connection to this host. Trust this key?'}
                </p>
                <p className="break-all font-mono text-xs">{shown.fingerprint}</p>
              </>
            ) : (
              <>
                <p className="text-destructive">
                  {shown.certificate
                    ? 'The certificate has changed - this may indicate a man-in-the-middle attack. Only accept if you know it was replaced.'
                    : 'The host key has changed - this may indicate a man-in-the-middle attack. Only accept if you know the key was rotated.'}
                </p>
                <p className="break-all font-mono text-xs">stored: {shown.stored}</p>
                <p className="break-all font-mono text-xs">offered: {shown.offered}</p>
              </>
            )}
          </div>
        )}
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={onReject}>
            Reject
          </Button>
          <Button type="button" onClick={onAccept}>
            {shown?.kind === 'mismatch'
              ? shown.certificate
                ? 'Accept changed certificate'
                : 'Accept changed key'
              : 'Trust'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
