import { ArrowLeft } from 'lucide-react'
import { cn } from '@/lib/utils'
import { type SettingsSection, useHostsUiStore } from '@/stores/hostsUiStore'

export const SECTIONS: [SettingsSection, string][] = [
  ['appearance', 'Appearance'],
  ['terminal', 'Terminal'],
  ['tags', 'Tags'],
  ['keyboard', 'Keyboard'],
  ['general', 'General'],
]

const row = 'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm'

export function SettingsNav({ onBack }: { onBack: () => void }) {
  const section = useHostsUiStore((s) => s.settingsSection)
  const openSettings = useHostsUiStore((s) => s.openSettings)
  return (
    <nav
      aria-label="Settings"
      className="flex w-full flex-col gap-1 overflow-y-auto bg-sidebar p-3"
    >
      <button type="button" onClick={onBack} className={cn(row, 'mb-2 hover:bg-muted')}>
        <ArrowLeft className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        Back to hosts
      </button>
      {SECTIONS.map(([id, label]) => (
        <button
          key={id}
          type="button"
          onClick={() => openSettings(id)}
          aria-current={section === id ? 'page' : undefined}
          className={cn(row, section === id ? 'bg-muted font-medium' : 'hover:bg-muted')}
        >
          {label}
        </button>
      ))}
    </nav>
  )
}
