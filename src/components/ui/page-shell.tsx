import type { ReactNode } from 'react'

export function PageShell({
  title,
  footer,
  children,
}: {
  title: ReactNode
  footer?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center border-border border-b px-4 py-2 font-semibold">
        {title}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">{children}</div>
      {footer && (
        <div className="flex shrink-0 justify-end gap-2 border-border border-t px-4 py-3">
          {footer}
        </div>
      )}
    </div>
  )
}
