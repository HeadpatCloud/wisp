import type { Ref } from 'react'

export function ConnectError({
  name,
  reason,
  onRetry,
  retryLabel = 'Retry',
  retryRef,
  action,
}: {
  name: string
  reason: string | null
  onRetry: () => void
  retryLabel?: string
  retryRef?: Ref<HTMLButtonElement>
  action?: { label: string; run: () => void }
}) {
  return (
    <div role="alert" className="flex max-w-sm flex-col items-center gap-2 text-center text-sm">
      <p className="font-medium">Couldn't connect to {name}</p>
      {reason && <p className="text-muted-foreground">{reason}</p>}
      <div className="flex gap-2">
        <button
          ref={retryRef}
          type="button"
          onClick={onRetry}
          className="rounded border border-border px-3 py-1.5 hover:bg-muted"
        >
          {retryLabel}
        </button>
        {action && (
          <button
            type="button"
            onClick={action.run}
            className="rounded border border-border px-3 py-1.5 hover:bg-muted"
          >
            {action.label}
          </button>
        )}
      </div>
    </div>
  )
}
