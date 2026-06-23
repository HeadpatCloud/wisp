import { commands } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export const importIcon = async (sourcePath: string): Promise<string> =>
  unwrap(await commands.importIcon(sourcePath))

export const readIcon = async (relPath: string): Promise<string> =>
  unwrap(await commands.readIcon(relPath))
