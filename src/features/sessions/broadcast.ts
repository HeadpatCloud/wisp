import { writeLocal } from '@/lib/local'
import { writeSession } from '@/lib/ssh'
import { useSessionStore } from '@/stores/sessionStore'

// With "Type into every pane of the tab" on, what is typed in one terminal pane goes to every
// live terminal pane of its tab, the typing one included. False when it's off.
export function typeInto(paneId: string, data: string): boolean {
  const st = useSessionStore.getState()
  const tab = st.tabs.find((t) => t.kind === 'panes' && t.paneIds.includes(paneId))
  if (tab?.kind !== 'panes' || !tab.broadcast) return false
  for (const id of tab.paneIds) {
    const p = st.panes[id]
    if (p?.kind === 'ssh' && p.sshId && p.status === 'connected') {
      writeSession(p.sshId, data).catch(() => {})
    } else if (p?.kind === 'local' && p.localId) {
      writeLocal(p.localId, data).catch(() => {})
    }
  }
  return true
}
