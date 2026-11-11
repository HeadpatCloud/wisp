import type { Tunnel } from '@/bindings'
import { connectErrorText } from '@/lib/ipc'
import { forwardLabel, openBackground, startTunnel } from '@/lib/tunnels'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { notify } from '@/stores/noticeStore'
import { useSessionStore } from '@/stores/sessionStore'
import { isRunning, useTunnelStore } from '@/stores/tunnelStore'

// The session of a connected terminal tab of the connection, if one is open.
export function tabSession(connectionId: string): string | null {
  const pane = Object.values(useSessionStore.getState().panes).find(
    (p) => p.kind === 'ssh' && p.connectionId === connectionId && p.status === 'connected',
  )
  return pane?.kind === 'ssh' ? (pane.sshId ?? null) : null
}

// Runs on an open tab's session, else on the connection's background connection, opened on
// first use. Throws only when the background connection can't be opened, so the caller can
// answer a host-key prompt.
export async function runTunnel(connectionId: string, tunnel: Tunnel): Promise<void> {
  const tunnels = useTunnelStore.getState()
  if (isRunning(tunnels.byId[tunnel.id])) return
  let sessionId = tabSession(connectionId) ?? tunnels.background[connectionId]
  if (!sessionId) {
    sessionId = await openBackground(connectionId)
    tunnels.setBackground(connectionId, sessionId)
  }
  await startOn(sessionId, tunnel)
}

export async function startOn(sessionId: string, tunnel: Tunnel): Promise<void> {
  const tunnels = useTunnelStore.getState()
  tunnels.start({ tunnelId: tunnel.id, sessionId, state: 'starting', bytesUp: 0, bytesDown: 0 })
  try {
    await startTunnel(sessionId, tunnel)
  } catch (e) {
    useTunnelStore.getState().setStatus({
      tunnelId: tunnel.id,
      state: 'error',
      bytesUp: 0,
      bytesDown: 0,
      message: connectErrorText(e),
    })
    tunnelNotice(tunnel, `Couldn't start: ${connectErrorText(e)}`)
  }
}

// An auto-started tunnel fails or stops where no Tunnels page is open, so it says so.
export function tunnelNotice(tunnel: Tunnel, reason: string) {
  notify({
    title: 'Tunnel stopped',
    body: `${forwardLabel(tunnel)}: ${reason}`,
    tone: 'error',
    actions: [{ label: 'Open Tunnels', run: () => useHostsUiStore.getState().openPage('tunnels') }],
  })
}
