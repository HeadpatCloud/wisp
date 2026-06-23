import { Fragment } from 'react'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import type { SessionTab } from '@/stores/sessionStore'
import { PaneView } from './PaneView'

export function PanesView({ tab }: { tab: SessionTab }) {
  return (
    <ResizablePanelGroup orientation={tab.direction}>
      {tab.sessionIds.map((id, i) => (
        <Fragment key={id}>
          {i > 0 && <ResizableHandle withHandle orientation={tab.direction} />}
          <ResizablePanel key={id} id={id} minSize="10%">
            <PaneView tabId={tab.id} sessionId={id} />
          </ResizablePanel>
        </Fragment>
      ))}
    </ResizablePanelGroup>
  )
}
