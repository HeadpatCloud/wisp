import { commands, type Tunnel } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export const startTunnel = async (sshId: string, tunnel: Tunnel): Promise<void> => {
  unwrap(await commands.tunnelStart(sshId, tunnel))
}

export const stopTunnel = async (tunnelId: string): Promise<void> => {
  unwrap(await commands.tunnelStop(tunnelId))
}

export const listTunnels = async (): Promise<string[]> => unwrap(await commands.tunnelList())
