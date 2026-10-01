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
  return (
    <Dialog open={prompt !== null} onOpenChange={(open) => !open && onReject()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {prompt?.kind === 'mismatch'
              ? prompt.certificate
                ? 'Certificate CHANGED'
                : 'Host key CHANGED'
              : prompt?.certificate
                ? 'Unknown certificate'
                : 'Unknown host key'}
          </DialogTitle>
        </DialogHeader>
        {prompt && (
          <div className="space-y-2 text-sm">
            <p className="text-muted-foreground">
              {prompt.certificate ? prompt.host.replace(/^[a-z]+\//, '') : prompt.host}:
              {prompt.port}
            </p>
            {prompt.kind === 'unknown' ? (
              <>
                <p>
                  {prompt.certificate
                    ? 'First connection to this server. Trust this certificate?'
                    : 'First connection to this host. Trust this key?'}
                </p>
                <p className="break-all font-mono text-xs">{prompt.fingerprint}</p>
              </>
            ) : (
              <>
                <p className="text-destructive">
                  {prompt.certificate
                    ? 'The certificate has changed - this may indicate a man-in-the-middle attack. Only accept if you know it was replaced.'
                    : 'The host key has changed - this may indicate a man-in-the-middle attack. Only accept if you know the key was rotated.'}
                </p>
                <p className="break-all font-mono text-xs">stored: {prompt.stored}</p>
                <p className="break-all font-mono text-xs">offered: {prompt.offered}</p>
              </>
            )}
          </div>
        )}
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={onReject}>
            Reject
          </Button>
          <Button type="button" onClick={onAccept}>
            {prompt?.kind === 'mismatch' ? 'Accept changed key' : 'Trust'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
