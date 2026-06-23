import { GripVertical } from 'lucide-react'
import type { ComponentProps } from 'react'
import { Group, Panel, Separator } from 'react-resizable-panels'
import { cn } from '@/lib/utils'

// Group sets display:flex + flexDirection from `orientation` itself; it just needs to fill
// its container. v4 has no orientation data-attribute, so the handle takes orientation directly.
export function ResizablePanelGroup({ className, ...props }: ComponentProps<typeof Group>) {
  return <Group className={cn('h-full w-full', className)} {...props} />
}

export const ResizablePanel = Panel

export function ResizableHandle({
  orientation = 'horizontal',
  withHandle,
  className,
  ...props
}: ComponentProps<typeof Separator> & {
  orientation?: 'horizontal' | 'vertical'
  withHandle?: boolean
}) {
  return (
    <Separator
      className={cn(
        'relative flex items-center justify-center bg-border after:absolute',
        orientation === 'vertical'
          ? 'h-px w-full cursor-row-resize after:inset-x-0 after:top-1/2 after:h-1 after:-translate-y-1/2'
          : 'w-px cursor-col-resize after:inset-y-0 after:left-1/2 after:w-1 after:-translate-x-1/2',
        className,
      )}
      {...props}
    >
      {withHandle && (
        <div className="z-10 flex h-4 w-3 items-center justify-center rounded-sm border border-border bg-border">
          <GripVertical className="size-2.5" />
        </div>
      )}
    </Separator>
  )
}
