import { commands, type Tunnel } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export const startTunnel = async (sshId: string, tunnel: Tunnel): Promise<void> => {
  unwrap(await commands.tunnelStart(sshId, tunnel))
}

export const stopTunnel = async (tunnelId: string): Promise<void> => {
  unwrap(await commands.tunnelStop(tunnelId))
}

export const listTunnels = async (): Promise<string[]> => unwrap(await commands.tunnelList())

// Throws the raw AppError, so a host-key prompt can be told by its kind.
export async function openBackground(connectionId: string): Promise<string> {
  const res = await commands.sshOpenBackground(connectionId)
  if (res.status === 'error') throw res.error
  return res.data
}

const KIND = { local: 'Local', remote: 'Remote', dynamic: 'SOCKS' } as const

export function forwardLabel(t: Tunnel): string {
  const bind = `${t.bindHost}:${t.bindPort}`
  return t.kind === 'dynamic'
    ? `${KIND.dynamic} ${bind}`
    : `${KIND[t.kind]} ${bind} → ${t.targetHost}:${t.targetPort}`
}
