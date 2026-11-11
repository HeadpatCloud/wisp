import { Fragment } from 'react'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { cn } from '@/lib/utils'
import { type PanesTab, type SplitDirection, useSessionStore } from '@/stores/sessionStore'
import { PaneSlot } from './PaneLayer'
import { useTabDrag } from './tabDrag'

const EDGES: { side: string; direction: SplitDirection; before: boolean; className: string }[] = [
  { side: 'left', direction: 'horizontal', before: true, className: 'inset-y-0 left-0 w-1/5' },
  { side: 'right', direction: 'horizontal', before: false, className: 'inset-y-0 right-0 w-1/5' },
  { side: 'top', direction: 'vertical', before: true, className: 'inset-x-0 top-0 h-1/5' },
  { side: 'bottom', direction: 'vertical', before: false, className: 'inset-x-0 bottom-0 h-1/5' },
]

function EdgeZones({ tabId }: { tabId: string }) {
  const dragged = useTabDrag((s) => s.tabId)
  const mergeable = useSessionStore(
    (s) => dragged !== tabId && s.tabs.find((t) => t.id === dragged)?.kind === 'panes',
  )
  if (!dragged || !mergeable) return null
  return (
    <>
      {EDGES.map((e) => (
        <div
          key={e.side}
          data-drop-edge={e.direction}
          data-drop-into={tabId}
          data-drop-before={e.before || undefined}
          className={cn('absolute z-30 rounded data-drop-over:bg-primary/20', e.className)}
        />
      ))}
    </>
  )
}

export function PanesView({ tab }: { tab: PanesTab }) {
  return (
    <div className="relative h-full">
      <ResizablePanelGroup orientation={tab.direction}>
        {tab.paneIds.map((id, i) => (
          <Fragment key={id}>
            {i > 0 && <ResizableHandle withHandle orientation={tab.direction} />}
            <ResizablePanel key={id} id={id} minSize="10%">
              <PaneSlot paneId={id} />
            </ResizablePanel>
          </Fragment>
        ))}
      </ResizablePanelGroup>
      <EdgeZones tabId={tab.id} />
    </div>
  )
}
