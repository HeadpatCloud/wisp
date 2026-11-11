import { HostEditorPage } from '@/features/connections/HostEditorPage'
import { NewConnectionPage } from '@/features/connections/NewConnectionPage'
import { SshImportPage } from '@/features/connections/SshImportPage'
import { KeysPage } from '@/features/keys/KeysPage'
import { KnownHostsPage } from '@/features/knownhosts/KnownHostsPage'
import { SettingsNav } from '@/features/settings/SettingsNav'
import { SettingsPage } from '@/features/settings/SettingsPage'
import { HostColumn } from '@/features/shell/HostColumn'
import { ExportPage } from '@/features/transfer/ExportPage'
import { ImportReviewPage } from '@/features/transfer/ImportReviewPage'
import { TunnelsPage } from '@/features/tunnels/TunnelsPage'
import { cn } from '@/lib/utils'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { HostsNav } from './HostsNav'
import { HostsPage } from './HostsPage'

export function HostsTab() {
  const page = useHostsUiStore((s) => s.page)
  const importPath = useHostsUiStore((s) => s.importPath)
  const newConnection = useHostsUiStore((s) => s.newConnection)
  const editHostId = useHostsUiStore((s) => s.editHostId)
  const editConnectionId = useHostsUiStore((s) => s.editConnectionId)
  const openPage = useHostsUiStore((s) => s.openPage)
  const right = useSettingsStore((s) => s.settings.hostListSide === 'right')
  const back = () => openPage('hosts')

  return (
    <div className={cn('flex h-full bg-background', right && 'flex-row-reverse')}>
      <HostColumn>{page === 'settings' ? <SettingsNav onBack={back} /> : <HostsNav />}</HostColumn>
      <div className="min-w-0 flex-1 overflow-auto">
        {page === 'hosts' && <HostsPage />}
        {page === 'settings' && <SettingsPage />}
        {page === 'import' && importPath && (
          <ImportReviewPage key={importPath} path={importPath} onClose={back} />
        )}
        {page === 'export' && <ExportPage onClose={back} />}
        {page === 'tunnels' && <TunnelsPage onClose={back} />}
        {page === 'keys' && <KeysPage onClose={back} />}
        {page === 'knownHosts' && <KnownHostsPage onClose={back} />}
        {page === 'sshImport' && <SshImportPage onClose={back} />}
        {page === 'edit' && editHostId && (
          <HostEditorPage
            key={editHostId}
            hostId={editHostId}
            openConnectionId={editConnectionId}
            onClose={back}
          />
        )}
        {page === 'new' && newConnection && (
          <NewConnectionPage
            key={JSON.stringify(newConnection)}
            prefill={newConnection}
            onClose={back}
          />
        )}
      </div>
    </div>
  )
}
