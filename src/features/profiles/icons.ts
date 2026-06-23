import { Cloud, Database, HardDrive, type LucideIcon, Server, Terminal } from 'lucide-react'
import type { IconRef } from '@/bindings'

export const BUILTIN_ICONS: { name: string; Icon: LucideIcon }[] = [
  { name: 'server', Icon: Server },
  { name: 'database', Icon: Database },
  { name: 'cloud', Icon: Cloud },
  { name: 'terminal', Icon: Terminal },
  { name: 'storage', Icon: HardDrive },
]

export function iconFor(ref: IconRef | undefined): LucideIcon {
  if (ref?.kind === 'builtin') {
    return BUILTIN_ICONS.find((i) => i.name === ref.name)?.Icon ?? Server
  }
  return Server
}
