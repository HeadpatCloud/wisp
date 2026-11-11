import type { Host, Tag } from '@/bindings'
import { findConnection, THIS_COMPUTER } from '@/lib/hosts'
import { rulesFor, type TagRules } from '@/lib/tagRules'
import { confirm } from '@/stores/confirmStore'
import { useHostStore } from '@/stores/hostStore'
import { type Pane, type Tab, useSessionStore } from '@/stores/sessionStore'

export function paneHost(p: Pane | undefined, hosts: Host[]): Host | undefined {
  if (!p) return undefined
  if (p.kind === 'local') return hosts.find((h) => h.id === THIS_COMPUTER)
  const id = p.connectionId ?? (p.kind === 'sftp' ? p.sshConnectionId : null)
  return id ? findConnection(hosts, id)?.host : undefined
}

// Every host a tab shows: each pane's, or a remote desktop's.
export function tabHosts(t: Tab, panes: Record<string, Pane>, hosts: Host[]): Host[] {
  if (t.kind !== 'panes') {
    const h = t.connectionId ? findConnection(hosts, t.connectionId)?.host : undefined
    return h ? [h] : []
  }
  return t.paneIds.flatMap((id) => paneHost(panes[id], hosts) ?? [])
}

// The rules of what the tab shows: its active pane's host gives the colour.
export function tabRules(
  t: Tab,
  panes: Record<string, Pane>,
  hosts: Host[],
  tags: Tag[],
): TagRules {
  const host =
    t.kind === 'panes' ? paneHost(panes[t.activePaneId], hosts) : tabHosts(t, panes, hosts)[0]
  return rulesFor(host, tags)
}

// The first host in the tab that asks before closing, with the tag that makes it ask.
export function closeQuestion(t: Tab): { host: Host; tag: string } | null {
  const { panes } = useSessionStore.getState()
  const { hosts, tags } = useHostStore.getState()
  for (const host of tabHosts(t, panes, hosts)) {
    const rules = rulesFor(host, tags)
    if (rules.askBeforeClose && rules.closeTag) return { host, tag: rules.closeTag }
  }
  return null
}

// The hosts whose open tabs ask before Wisp closes, by name; empty when none does.
export function hostsAskingToClose(): string {
  const asking = useSessionStore
    .getState()
    .tabs.map(closeQuestion)
    .filter((q): q is NonNullable<typeof q> => !!q)
  return [...new Set(asking.map((q) => q.host.name))].join(', ')
}

// Every way of closing a tab comes here, so a tag that asks first is never skipped.
export async function requestCloseTab(tabId: string): Promise<void> {
  const tab = useSessionStore.getState().tabs.find((t) => t.id === tabId)
  const q = tab ? closeQuestion(tab) : null
  if (
    q &&
    !(await confirm({
      title: `Close ${q.host.name}?`,
      body: `This tab is tagged ${q.tag}. Anything still running in the shell stops.`,
      confirmLabel: 'Close tab',
      destructive: true,
    }))
  ) {
    return
  }
  useSessionStore.getState().removeTab(tabId)
}

// Closing the last pane closes the tab, so it asks the same way.
export async function requestClosePane(tabId: string, paneId: string): Promise<void> {
  const tab = useSessionStore.getState().tabs.find((t) => t.id === tabId)
  if (tab?.kind === 'panes' && tab.paneIds.length === 1) return requestCloseTab(tabId)
  useSessionStore.getState().closePane(tabId, paneId)
}

// Asked when the paste happens: a host that asks first, or, while typing goes to every pane of
// the tab, any terminal pane's host that does.
export function pasteAsks(paneId: string): boolean {
  const { tabs, panes } = useSessionStore.getState()
  const { hosts, tags } = useHostStore.getState()
  const tab = tabs.find((t) => t.kind === 'panes' && t.paneIds.includes(paneId))
  const ids = tab?.kind === 'panes' && tab.broadcast ? tab.paneIds : [paneId]
  return ids.some((id) => {
    const p = panes[id]
    return (
      (p?.kind === 'ssh' || p?.kind === 'local') &&
      rulesFor(paneHost(p, hosts), tags).askBeforePaste
    )
  })
}
