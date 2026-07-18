import { getCurrentWindow } from '@tauri-apps/api/window'
import { message } from '@tauri-apps/plugin-dialog'
import { Settings } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { events, type S3Profile, type ShellInfo } from '@/bindings'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { FtpConnectDialog } from '@/features/ftp/FtpConnectDialog'
import { FtpConnectionView } from '@/features/ftp/FtpConnectionView'
import { CommandPalette } from '@/features/palette/CommandPalette'
import { ProfileTree } from '@/features/profiles/ProfileTree'
import { S3ConnectionView } from '@/features/s3/S3ConnectionView'
import { S3ProfileDialog } from '@/features/s3/S3ProfileDialog'
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
import { useHotkeys } from '@/lib/hotkeys'
import { clearEditTemp, listShells } from '@/lib/local'
import { exportProfilesToFile, importProfilesFromFile } from '@/lib/profiles'
import { clearSnapshot, loadSnapshot, saveSnapshot } from '@/lib/sessionPersist'
import { watchSystemTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { setSecret } from '@/lib/vault'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import {
  type FtpTab,
  type LocalTab,
  type S3Tab,
  type SessionTab,
  type SftpTab,
  useSessionStore,
  type VncTab,
} from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { useTunnelStore } from '@/stores/tunnelStore'

function cycleTab(delta: number) {
  const st = useSessionStore.getState()
  if (st.tabs.length === 0) return
  const i = st.tabs.findIndex((t) => t.id === st.activeTabId)
  const next = st.tabs[(i + delta + st.tabs.length) % st.tabs.length]
  if (next) st.setActiveTab(next.id)
}

function splitActivePane(direction: 'horizontal' | 'vertical') {
  const st = useSessionStore.getState()
  const t = st.tabs.find((tab) => tab.id === st.activeTabId)
  if (t?.kind === 'session') st.splitPane(t.id, t.activePaneId, direction)
}

function nudgeZoom(delta: number, reset = false) {
  const st = useSessionStore.getState()
  const t = st.tabs.find((tab) => tab.id === st.activeTabId)
  if (t?.kind !== 'session') return
  const cur = st.sessions[t.activePaneId]?.zoom ?? 0
  st.setZoom(t.activePaneId, reset ? 0 : cur + delta)
}

export default function App() {
  const load = useProfileStore((s) => s.load)
  const tabs = useSessionStore((s) => s.tabs)
  const activeTabId = useSessionStore((s) => s.activeTabId)
  const openTab = useSessionStore((s) => s.openTab)
  const openView = useSessionStore((s) => s.openView)
  const openLocalShell = useSessionStore((s) => s.openLocalShell)
  const openVnc = useSessionStore((s) => s.openVnc)
  const openSftp = useSessionStore((s) => s.openSftp)
  const openSftpAdhoc = useSessionStore((s) => s.openSftpAdhoc)
  const openFtp = useSessionStore((s) => s.openFtp)
  const openS3 = useSessionStore((s) => s.openS3)
  const loadSettings = useSettingsStore((s) => s.load)
  const loadS3 = useS3ProfileStore((s) => s.load)
  const themeValue = useSettingsStore((s) => s.settings.theme)
  const settingsHotkeys = useSettingsStore((s) => s.settings.hotkeys ?? {})
  const [vncDialogOpen, setVncDialogOpen] = useState(false)
  const [ftpDialogOpen, setFtpDialogOpen] = useState(false)
  const [sftpDialogOpen, setSftpDialogOpen] = useState(false)
  const [s3DialogOpen, setS3DialogOpen] = useState(false)
  const [s3Editing, setS3Editing] = useState<S3Profile | null>(null)
  const [shells, setShells] = useState<ShellInfo[]>([])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [vaultReady, setVaultReady] = useState(false)
  const settingsLoaded = useSettingsStore((s) => s.loaded)
  const restoreEnabled = useSettingsStore((s) => s.settings.restoreSession ?? true)
  const restoredRef = useRef(false)
  const restoreEnabledRef = useRef(restoreEnabled)
  restoreEnabledRef.current = restoreEnabled

  // Restoring waits on both the vault (secrets) and settings (the toggle still reads as its
  // default until they load), and persistence only starts afterwards so an empty startup
  // state can't overwrite the saved snapshot.
  useEffect(() => {
    if (!vaultReady || !settingsLoaded || restoredRef.current) return
    restoredRef.current = true
    if (restoreEnabledRef.current) {
      const snap = loadSnapshot()
      // Anything opened while the vault was resolving must survive.
      if (snap && useSessionStore.getState().tabs.length === 0) {
        useSessionStore.getState().restoreTabs(snap)
      }
    } else {
      clearSnapshot()
    }
    return useSessionStore.subscribe((s) => {
      if (restoreEnabledRef.current) saveSnapshot(s)
    })
  }, [vaultReady, settingsLoaded])

  useHotkeys(
    {
      palette: () => setPaletteOpen((v) => !v),
      localShell: () => openLocalShell(),
      settings: () => openView({ kind: 'settings' }, 'Settings'),
      closeTab: () => {
        const st = useSessionStore.getState()
        if (st.activeTabId) st.removeTab(st.activeTabId)
      },
      nextTab: () => cycleTab(1),
      prevTab: () => cycleTab(-1),
      splitRight: () => splitActivePane('horizontal'),
      splitDown: () => splitActivePane('vertical'),
      closePane: () => {
        const st = useSessionStore.getState()
        const t = st.tabs.find((tab) => tab.id === st.activeTabId)
        if (t?.kind === 'session') st.closePane(t.id, t.activePaneId)
      },
      zoomIn: () => nudgeZoom(1),
      zoomOut: () => nudgeZoom(-1),
      zoomReset: () => nudgeZoom(0, true),
      broadcast: () => {
        const st = useSessionStore.getState()
        const t = st.tabs.find((tab) => tab.id === st.activeTabId)
        if (t?.kind === 'session') st.toggleBroadcast(t.id)
      },
    },
    settingsHotkeys,
  )

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
    clearEditTemp().catch(console.error)
  }, [])
  useEffect(() => {
    loadSettings().catch(console.error)
  }, [loadSettings])
  useEffect(() => {
    loadS3().catch(console.error)
  }, [loadS3])
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
      // Off by default: a remote VNC server shouldn't silently write to the local clipboard
      if (!useSettingsStore.getState().settings.vncClipboardSync) return
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
    await loadS3()
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
  const s3Tabs = tabs.filter((t): t is S3Tab => t.kind === 's3')
  const activeViewTab = activeTab && activeTab.kind === 'view' ? activeTab : null

  return (
    <>
      <VaultGate onReady={() => setVaultReady(true)} />
      <UpdateBanner />
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
      <VncConnectDialog
        open={vncDialogOpen}
        onOpenChange={setVncDialogOpen}
        onConnect={async (host, port, password) =>
          openVnc(host, port, password ? await setSecret(password) : null)
        }
      />
      <FtpConnectDialog
        open={ftpDialogOpen}
        onOpenChange={setFtpDialogOpen}
        onConnect={async ({ password, ...rest }) =>
          openFtp({ ...rest, secretId: password ? await setSecret(password) : null })
        }
      />
      <SftpConnectDialog
        open={sftpDialogOpen}
        onOpenChange={setSftpDialogOpen}
        onPick={openSftp}
        onConnect={async ({ secret, ...rest }) =>
          openSftpAdhoc({ ...rest, secretId: secret ? await setSecret(secret) : null })
        }
      />
      <S3ProfileDialog open={s3DialogOpen} onOpenChange={setS3DialogOpen} editing={s3Editing} />
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
                onNewS3={() => {
                  setS3Editing(null)
                  setS3DialogOpen(true)
                }}
                onActivateS3={(p) => openS3(p.id, p.bucket, p.name)}
                onEditS3={(p) => {
                  setS3Editing(p)
                  setS3DialogOpen(true)
                }}
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
                  <VncView host={t.host} port={t.port} secretId={t.secretId} />
                </div>
              ))}
              {sftpTabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <SftpConnectionView
                    profileId={t.profileId}
                    adhoc={t.adhoc}
                    active={t.id === activeTabId}
                  />
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
                    secretId={t.secretId}
                    secure={t.secure}
                    allowInvalidCert={t.allowInvalidCert}
                    ignoreHostname={t.ignoreHostname}
                    active={t.id === activeTabId}
                  />
                </div>
              ))}
              {s3Tabs.map((t) => (
                <div
                  key={t.id}
                  data-testid={`tabpane-${t.id}`}
                  className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                >
                  <S3ConnectionView
                    profileId={t.profileId}
                    bucket={t.bucket}
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
