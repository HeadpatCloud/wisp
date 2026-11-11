import {
  Cloud,
  Database,
  FolderOpen,
  HardDrive,
  type LucideIcon,
  Monitor,
  Server,
  SquareTerminal,
  Terminal,
} from 'lucide-react'
import type { IconRef } from '@/bindings'
import type { typeGroup } from '@/lib/hosts'

export const GROUP_ICON: Record<ReturnType<typeof typeGroup>, LucideIcon> = {
  terminal: SquareTerminal,
  files: FolderOpen,
  desktop: Monitor,
}

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
