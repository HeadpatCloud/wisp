import { getCurrentWindow } from '@tauri-apps/api/window'
import { message } from '@tauri-apps/plugin-dialog'
import { Settings } from 'lucide-react'
import { useEffect, useState } from 'react'
import { events, type ShellInfo } from '@/bindings'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { FtpConnectDialog } from '@/features/ftp/FtpConnectDialog'
import { FtpConnectionView } from '@/features/ftp/FtpConnectionView'
import { ProfileTree } from '@/features/profiles/ProfileTree'
import { LocalTerminalView } from '@/features/sessions/LocalTerminalView'
import { PanesView } from '@/features/sessions/PanesView'
import { SftpConnectionView } from '@/features/sessions/SftpConnectionView'
import { TabBar } from '@/features/sessions/TabBar'
import { ViewHost } from '@/features/sessions/ViewHost'
import { VncConnectDialog } from '@/features/sessions/VncConnectDialog'
import { VncView } from '@/features/sessions/VncView'
import { SftpConnectDialog } from '@/features/sftp/SftpConnectDialog'
import { AppShell } from '@/features/shell/AppShell'
import { UpdateBanner } from '@/features/updater/UpdateBanner'
import { VaultGate } from '@/features/vault/VaultGate'
import { WelcomePage } from '@/features/welcome/WelcomePage'
import { listShells } from '@/lib/local'
import { exportProfilesToFile, importProfilesFromFile } from '@/lib/profiles'
import { watchSystemTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { useProfileStore } from '@/stores/profileStore'
import {
  type FtpTab,
  type LocalTab,
  type SessionTab,
  type SftpTab,
  useSessionStore,
  type VncTab,
} from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { useTunnelStore } from '@/stores/tunnelStore'

export default function App() {
  const load = useProfileStore((s) => s.load)
  const tabs = useSessionStore((s) => s.tabs)
  const activeTabId = useSessionStore((s) => s.activeTabId)
  const openTab = useSessionStore((s) => s.openTab)
  const openView = useSessionStore((s) => s.openView)
  const openLocalShell = useSessionStore((s) => s.openLocalShell)
  const openVnc = useSessionStore((s) => s.openVnc)
  const openSftp = useSessionStore((s) => s.openSftp)
  const openFtp = useSessionStore((s) => s.openFtp)
  const loadSettings = useSettingsStore((s) => s.load)
  const themeValue = useSettingsStore((s) => s.settings.theme)
  const [vncDialogOpen, setVncDialogOpen] = useState(false)
  const [ftpDialogOpen, setFtpDialogOpen] = useState(false)
  const [sftpDialogOpen, setSftpDialogOpen] = useState(false)
  const [shells, setShells] = useState<ShellInfo[]>([])

  useEffect(() => {
    load().catch(console.error)
  }, [load])
  useEffect(() => {
    try {
      getCurrentWindow()
        .setTitle(`Wisp v${__APP_VERSION__}`)
        .catch(() => {})
    } catch {
      // not running inside a Tauri window (tests / web preview)
    }
  }, [])
  useEffect(() => {
    listShells().then(setShells).catch(console.error)
  }, [])
  useEffect(() => {
    loadSettings().catch(console.error)
  }, [loadSettings])
  useEffect(() => {
    return watchSystemTheme(() =>
      themeValue === 'light' || themeValue === 'dark' ? themeValue : 'system',
    )
  }, [themeValue])
  useEffect(() => {
    const un = events.tunnelStatus.listen((e) =>
      useTunnelStore.getState().setStatus({
        ...e.payload,
        state: e.payload.state as import('@/stores/tunnelStore').TunnelState,
      }),
    )
    return () => {
      un.then((f) => f())
    }
  }, [])
  useEffect(() => {
    const un = events.vncClipboard.listen((e) => {
      navigator.clipboard.writeText(e.payload.text).catch(() => {})
    })
    return () => {
      un.then((f) => f())
    }
  }, [])

  const importFromFile = async () => {
    const n = await importProfilesFromFile()
    if (n === null) return
    await load()
    await message(`Imported ${n} profile${n === 1 ? '' : 's'}.`)
  }
  const exportToFile = async () => {
    if (await exportProfilesToFile()) await message('Profiles exported.')
  }

  const activeTab = tabs.find((t) => t.id === activeTabId)
  const sessionTabs = tabs.filter((t): t is SessionTab => t.kind === 'session')
  const localTabs = tabs.filter((t): t is LocalTab => t.kind === 'local')
  const vncTabs = tabs.filter((t): t is VncTab => t.kind === 'vnc')
  const sftpTabs = tabs.filter((t): t is SftpTab => t.kind === 'sftp')
  const ftpTabs = tabs.filter((t): t is FtpTab => t.kind === 'ftp')
  const activeViewTab = activeTab && activeTab.kind === 'view' ? activeTab : null

  return (
    <>
      <VaultGate />
      <UpdateBanner />
      <VncConnectDialog open={vncDialogOpen} onOpenChange={setVncDialogOpen} onConnect={openVnc} />
      <FtpConnectDialog open={ftpDialogOpen} onOpenChange={setFtpDialogOpen} onConnect={openFtp} />
      <SftpConnectDialog open={sftpDialogOpen} onOpenChange={setSftpDialogOpen} onPick={openSftp} />
      <AppShell
        sidebar={
          <div className="flex h-full flex-col">
            <div className="flex justify-end gap-1 border-border border-b p-1">
              <button
                type="button"
                aria-label="Home"
                onClick={() => {
                  // With no active tab the welcome screen is already shown as the empty
                  // state, so don't open a redundant Welcome tab.
                  if (activeTab) openView({ kind: 'welcome' }, 'Welcome')
                }}
                className="rounded px-2 py-1 text-xs hover:bg-muted"
              >
                Home
              </button>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    aria-label="Import"
                    className="rounded px-2 py-1 text-xs hover:bg-muted"
                  >
                    Import
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onSelect={() => openView({ kind: 'import' }, 'Import')}>
                    From SSH config
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={importFromFile}>From file (JSON)</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <button
                type="button"
                aria-label="Export profiles"
                className="rounded px-2 py-1 text-xs hover:bg-muted"
                onClick={exportToFile}
              >
                Export
              </button>
              <button
                type="button"
                aria-label="Settings"
                onClick={() => openView({ kind: 'settings' }, 'Settings')}
                className="rounded p-1.5 hover:bg-muted"
              >
                <Settings className="size-4" />
              </button>
            </div>
            <div className="min-h-0 flex-1">
              <ProfileTree
                onActivateProfile={(p) =>
                  openTab({
                    id: crypto.randomUUID(),
                    profileId: p.id,
                    title: p.name,
                    status: 'connecting',
                    reconnectNonce: 0,
                  })
                }
                onNewProfile={() =>
                  openView({ kind: 'profile-editor', profileId: null }, 'New profile')
                }
                onNewVnc={() => setVncDialogOpen(true)}
                onNewFtp={() => setFtpDialogOpen(true)}
                onNewLocalShell={(program, title) => openLocalShell(program, title)}
                onNewSftp={(profileId, title) => openSftp(profileId, title)}
                onOpenSftpPicker={() => setSftpDialogOpen(true)}
                shells={shells}
                onEditProfile={(profile) =>
                  openView(
                    { kind: 'profile-editor', profileId: profile.id },
                    `Edit ${profile.name}`,
                  )
                }
                onNewGroup={() => openView({ kind: 'group-editor', groupId: null }, 'New group')}
                onEditGroup={(group) =>
                  openView({ kind: 'group-editor', groupId: group.id }, `Edit ${group.name}`)
                }
              />
            </div>
          </div>
        }
        main={
          <div className="flex h-full flex-col">
            <TabBar />
            <div className="relative min-h-0 flex-1">
              {sessionTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <PanesView tab={t} />
                </div>
              ))}
              {localTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <LocalTerminalView program={t.program} />
                </div>
              ))}
              {vncTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <VncView host={t.host} port={t.port} password={t.password} />
                </div>
              ))}
              {sftpTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <SftpConnectionView profileId={t.profileId} active={t.id === activeTabId} />
                </div>
              ))}
              {ftpTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <FtpConnectionView
                    host={t.host}
                    port={t.port}
                    username={t.username}
                    password={t.password}
                    secure={t.secure}
                    allowInvalidCert={t.allowInvalidCert}
                    active={t.id === activeTabId}
                  />
                </div>
              ))}
              {activeViewTab && (
                <div className="absolute inset-0 bg-background">
                  <ViewHost key={activeViewTab.id} tab={activeViewTab} />
                </div>
              )}
              {!activeTab && (
                <div className="absolute inset-0">
                  <WelcomePage />
                </div>
              )}
            </div>
          </div>
        }
      />
    </>
  )
}
