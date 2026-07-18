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

// Ad-hoc connections (FTP, VNC, manual SFTP) park their password here so the tab holds a
// reference instead of the plaintext, which also makes those tabs safe to persist.
export async function setSecret(value: string): Promise<string> {
  return unwrap(await commands.setSecret(value))
}

export async function deleteSecret(id: string): Promise<void> {
  unwrap(await commands.deleteSecret(id))
}
