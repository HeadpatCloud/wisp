import { X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { type Notice, useNoticeStore } from '@/stores/noticeStore'

export function Notices() {
  const notices = useNoticeStore((s) => s.notices)
  if (notices.length === 0) return null
  return (
    <div className="fixed right-4 bottom-4 z-50 flex w-96 max-w-full flex-col gap-2">
      {notices.map((n) => (
        <NoticeCard key={n.id} notice={n} />
      ))}
    </div>
  )
}

function NoticeCard({ notice }: { notice: Notice }) {
  const { act, dismiss, hold, release } = useNoticeStore.getState()
  const error = notice.tone === 'error'
  return (
    <div
      role={error ? 'alert' : 'status'}
      onPointerEnter={() => hold(notice.id)}
      onPointerLeave={() => release(notice.id)}
      className={cn(
        'group overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-lg',
        error ? 'border-destructive' : 'border-border',
      )}
    >
      <div className="flex flex-wrap items-start gap-2 p-3 pl-4">
        <div className="min-w-48 flex-1">
          <p className="font-semibold text-sm">{notice.title}</p>
          {notice.body && (
            <p className="mt-0.5 whitespace-pre-line text-muted-foreground text-sm">
              {notice.body}
            </p>
          )}
        </div>
        <div className="flex items-center gap-1">
          {notice.actions?.map((a, i) => (
            <Button
              key={a.label}
              size="sm"
              variant={i === 0 ? 'default' : 'ghost'}
              onClick={() => act(notice.id, a)}
            >
              {a.label}
            </Button>
          ))}
          <button
            type="button"
            aria-label="Dismiss"
            onClick={() => dismiss(notice.id)}
            className="rounded p-1 text-muted-foreground hover:bg-muted"
          >
            <X className="size-3.5" aria-hidden />
          </button>
        </div>
      </div>
      {!notice.sticky && (
        <div className="h-0.5 bg-muted" aria-hidden>
          <div className="h-full animate-notice bg-primary group-hover:paused" />
        </div>
      )}
    </div>
  )
}
