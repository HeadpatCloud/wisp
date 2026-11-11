import { PageShell } from '@/components/ui/page-shell'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { AppearancePage } from './AppearancePage'
import { GeneralPage } from './GeneralPage'
import { KeyboardPage } from './KeyboardPage'
import { SECTIONS } from './SettingsNav'
import { TagsPage } from './TagsPage'
import { TerminalPage } from './TerminalPage'

export function SettingsPage() {
  const section = useHostsUiStore((s) => s.settingsSection)
  const title = SECTIONS.find(([id]) => id === section)?.[1] ?? 'Settings'
  return (
    <PageShell title={title}>
      {section === 'appearance' && <AppearancePage />}
      {section === 'terminal' && <TerminalPage />}
      {section === 'tags' && <TagsPage />}
      {section === 'keyboard' && <KeyboardPage />}
      {section === 'general' && <GeneralPage />}
    </PageShell>
  )
}
