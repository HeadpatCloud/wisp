import { formatBytes } from '@/lib/format'
import { cancelTransfer } from '@/lib/transfers'
import { useTransferStore } from '@/stores/transferStore'

export function TransfersBar() {
  const transfers = useTransferStore((s) => s.transfers)
  if (transfers.length === 0) return null
  return (
    <div className="shrink-0 space-y-1 border-border border-t p-2">
      {transfers.map((t) => {
        const pct = t.total ? Math.round((t.transferred / t.total) * 100) : 0
        return (
          <div key={t.id} className="text-xs">
            <div className="flex justify-between gap-2">
              <span className="truncate">
                {t.dir === 'upload' ? '↑' : '↓'} {t.name}
              </span>
              <span className="flex shrink-0 items-center gap-1 text-muted-foreground">
                {t.status === 'error'
                  ? 'error'
                  : t.total > 0
                    ? `${pct}% - ${formatBytes(t.transferred)} / ${formatBytes(t.total)}`
                    : `${pct}%`}
                {t.status === 'active' && (
                  <button
                    type="button"
                    aria-label={`Cancel ${t.name}`}
                    onClick={() => cancelTransfer(t.id).catch(() => {})}
                    className="rounded px-1 hover:bg-muted"
                  >
                    ✕
                  </button>
                )}
              </span>
            </div>
            <div className="h-1 rounded bg-muted">
              <div
                className="h-1 rounded bg-primary transition-[width]"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>
        )
      })}
    </div>
  )
}
