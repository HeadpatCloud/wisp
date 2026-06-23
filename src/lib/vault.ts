import { commands, type VaultStatus } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export async function vaultStatus(): Promise<VaultStatus> {
  return unwrap(await commands.vaultStatus())
}

export async function vaultUnlock(password: string): Promise<void> {
  unwrap(await commands.vaultUnlock(password))
}

export async function vaultChangePassword(password: string): Promise<void> {
  unwrap(await commands.vaultChangePassword(password))
}
