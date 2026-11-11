import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useMemo, useRef, useState } from 'react'
import { commands, events } from '@/bindings'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Notices } from '@/components/Notices'
import { useConnectOnce } from '@/features/connections/connectOnce'
import { FtpConnectDialog } from '@/features/ftp/FtpConnectDialog'
import { HostsTab } from '@/features/hosts/HostsTab'
import { openConnection } from '@/features/hosts/openConnection'
import { CommandPalette } from '@/features/palette/CommandPalette'
import { RdpConnectDialog } from '@/features/remote/RdpConnectDialog'
import { RemoteDesktopView } from '@/features/remote/RemoteDesktopView'
import { toAdhocParams } from '@/features/sessions/AdhocLoginFields'
import { PaneLayer } from '@/features/sessions/PaneLayer'
import { PanesView } from '@/features/sessions/PanesView'
import { SessionHostList } from '@/features/sessions/SessionHostList'
import { SshConnectDialog } from '@/features/sessions/SshConnectDialog'
import { TabBar } from '@/features/sessions/TabBar'
import {
  hostsAskingToClose,
  requestClosePane,
  requestCloseTab,
  tabRules,
} from '@/features/sessions/tabHosts'
import { VncConnectDialog } from '@/features/sessions/VncConnectDialog'
import { SftpConnectDialog } from '@/features/sftp/SftpConnectDialog'
import { AppShell } from '@/features/shell/AppShell'
import { importFromFile } from '@/features/transfer/importFromFile'
import { tunnelNotice } from '@/features/tunnels/run'
import { useTunnelStarter } from '@/features/tunnels/useTunnelStarter'
import { startUpdateCheck } from '@/features/updater/updateNotice'
import { VaultGate } from '@/features/vault/VaultGate'
import { THIS_COMPUTER } from '@/lib/hosts'
import { useHotkeys } from '@/lib/hotkeys'
import { unwrap } from '@/lib/ipc'
import { clearEditTemp } from '@/lib/local'
import { customTitleBar } from '@/lib/platform'
import { rdpDriver } from '@/lib/rdp'
import { clearSnapshot, dropMissing, loadSnapshot, saveSnapshot } from '@/lib/sessionPersist'
import { tagColor } from '@/lib/tagColors'
import { watchSystemTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { setSecret } from '@/lib/vault'
import { vncDriver } from '@/lib/vnc'
import { confirm } from '@/stores/confirmStore'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { notify } from '@/stores/noticeStore'
import { type PanesTab, type RdpTab, useSessionStore, type VncTab } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { applyAppearance, useThemeStore } from '@/stores/themeStore'
import { type TunnelState, useTunnelStore } from '@/stores/tunnelStore'

const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`

// The pinned Hosts tab (null) comes first.
function cycleTab(delta: number) {
  const st = useSessionStore.getState()
  const ids = [null, ...st.tabs.map((t) => t.id)]
  const i = ids.indexOf(st.activeTabId)
  st.setActiveTab(ids[(i + delta + ids.length) % ids.length])
}

function moveActiveTab(delta: number) {
  const st = useSessionStore.getState()
  if (st.activeTabId) st.moveTab(st.activeTabId, delta)
}

function splitActivePane(direction: 'horizontal' | 'vertical') {
  const st = useSessionStore.getState()
  const t = st.tabs.find((tab) => tab.id === st.activeTabId)
  if (t?.kind === 'panes') st.splitPane(t.id, t.activePaneId, direction)
}

function nudgeZoom(delta: number, reset = false) {
  const st = useSessionStore.getState()
  const t = st.tabs.find((tab) => tab.id === st.activeTabId)
  if (t?.kind !== 'panes') return
  const pane = st.panes[t.activePaneId]
  if (pane?.kind !== 'ssh' && pane?.kind !== 'local') return
  st.setZoom(pane.id, reset ? 0 : (pane.zoom ?? 0) + delta)
}

// The view reconnects whenever it is given another driver object. A saved connection is
// resolved by the backend from its id, so the target fields are only for connect once.
function VncTabView({ tab, active }: { tab: VncTab; active: boolean }) {
  const { connectionId, once } = tab
  const driver = useMemo(
    () =>
      vncDriver({
        host: once?.host ?? '',
        port: once?.port ?? 0,
        username: once?.username ?? null,
        secretId: once?.secretId ?? null,
        profileId: connectionId,
      }),
    [connectionId, once],
  )
  return <RemoteDesktopView tabId={tab.id} driver={driver} active={active} />
}

function RdpTabView({ tab, active }: { tab: RdpTab; active: boolean }) {
  const { connectionId, once } = tab
  const driver = useMemo(
    () =>
      rdpDriver({
        host: once?.host ?? '',
        port: once?.port ?? 0,
        username: once?.username ?? '',
        domain: once?.domain ?? null,
        secretId: once?.secretId ?? null,
        profileId: connectionId,
        keyboardLayout: once?.keyboardLayout ?? null,
      }),
    [connectionId, once],
  )
  return <RemoteDesktopView tabId={tab.id} driver={driver} active={active} />
}

export default function App() {
  const loadHosts = useHostStore((s) => s.load)
  const loadShells = useHostStore((s) => s.loadShells)
  const tabs = useSessionStore((s) => s.tabs)
  const panes = useSessionStore((s) => s.panes)
  const hostsList = useHostStore((s) => s.hosts)
  const tagList = useHostStore((s) => s.tags)
  const hostListRight = useSettingsStore((s) => s.settings.hostListSide === 'right')
  const activeTabId = useSessionStore((s) => s.activeTabId)
  const openPage = useHostsUiStore((s) => s.openPage)
  const openLocalShell = useSessionStore((s) => s.openLocalShell)
  const openVnc = useSessionStore((s) => s.openVnc)
  const openRdp = useSessionStore((s) => s.openRdp)
  const openSftp = useSessionStore((s) => s.openSftp)
  const openSftpAdhoc = useSessionStore((s) => s.openSftpAdhoc)
  const openSshAdhoc = useSessionStore((s) => s.openSshAdhoc)
  const openFtp = useSessionStore((s) => s.openFtp)
  const loadSettings = useSettingsStore((s) => s.load)
  const settingsHotkeys = useSettingsStore((s) => s.settings.hotkeys ?? {})
  const [vncDialogOpen, setVncDialogOpen] = useState(false)
  const [rdpDialogOpen, setRdpDialogOpen] = useState(false)
  const [ftpDialogOpen, setFtpDialogOpen] = useState(false)
  const [sftpDialogOpen, setSftpDialogOpen] = useState(false)
  const [sshDialogOpen, setSshDialogOpen] = useState(false)
  const once = useConnectOnce((s) => s.request)
  const clearOnce = useConnectOnce((s) => s.clear)
  const onceInitial = useMemo(
    () =>
      once && {
        host: once.host,
        port: once.port ?? undefined,
        username: once.user ?? undefined,
        secure: once.type === 'ftps',
      },
    [once],
  )
  const onceFor = (...types: string[]) =>
    once && types.includes(once.type) ? (onceInitial ?? undefined) : undefined
  const closing = (set: (open: boolean) => void) => (open: boolean) => {
    set(open)
    if (!open) clearOnce()
  }
  const [paletteOpen, setPaletteOpen] = useState(false)
  const tunnelStarter = useTunnelStarter()
  const [vaultReady, setVaultReady] = useState(false)
  const settingsLoaded = useSettingsStore((s) => s.loaded)
  const hostsLoaded = useHostStore((s) => s.loaded)
  const restoreEnabled = useSettingsStore((s) => s.settings.restoreSession ?? true)
  const restoredRef = useRef(false)
  const restoreEnabledRef = useRef(restoreEnabled)
  restoreEnabledRef.current = restoreEnabled

  // Restoring waits on the vault (secrets), settings (the toggle still reads as its default
  // until they load) and hosts (to drop tabs of deleted connections), and persistence only
  // starts afterwards so an empty startup state can't overwrite the saved snapshot.
  useEffect(() => {
    if (!vaultReady || !settingsLoaded || !hostsLoaded || restoredRef.current) return
    restoredRef.current = true
    if (restoreEnabledRef.current) {
      const snap = loadSnapshot()
      // Anything opened while the vault was resolving must survive.
      if (snap && useSessionStore.getState().tabs.length === 0) {
        const ids = new Set(
          useHostStore.getState().hosts.flatMap((h) => h.connections.map((c) => c.id)),
        )
        useSessionStore.getState().restoreTabs(dropMissing(snap, ids))
      }
    } else {
      clearSnapshot()
    }
    return useSessionStore.subscribe((s) => {
      if (restoreEnabledRef.current) saveSnapshot(s)
    })
  }, [vaultReady, settingsLoaded, hostsLoaded])

  // The backend hands the report out once; StrictMode's second run gets null and leaves it.
  useEffect(() => {
    if (!vaultReady) return
    commands
      .takeMigrationReport()
      .then((res) => {
        const report = unwrap(res)
        if (!report) return
        notify({
          title: `${count(report.profiles, 'profile')} became ${count(report.hosts, 'host')}`,
          body: "Profiles with the same address were joined into one host. Your old profiles file is kept next to the new one.\nSomething joined that shouldn't be? Use Move to another host on that connection.",
          sticky: true,
          actions: [{ label: 'OK', run: () => {} }],
        })
      })
      .catch(console.error)
  }, [vaultReady])

  useEffect(() => {
    startUpdateCheck().catch(console.error)
  }, [])

  useHotkeys(
    {
      palette: () => setPaletteOpen((v) => !v),
      paletteShort: () => setPaletteOpen((v) => !v),
      newConnection: () => useHostsUiStore.getState().openNewConnection(),
      toggleHostList: () => useHostsUiStore.getState().toggleHostList(),
      localShell: () => {
        const pc = useHostStore.getState().hosts.find((h) => h.id === THIS_COMPUTER)
        const first = pc?.connections[0]
        if (pc && first) openConnection(pc, first)
        else openLocalShell()
      },
      settings: () => openPage('settings'),
      closeTab: () => {
        const st = useSessionStore.getState()
        if (st.activeTabId) void requestCloseTab(st.activeTabId)
      },
      nextTab: () => cycleTab(1),
      prevTab: () => cycleTab(-1),
      moveTabLeft: () => moveActiveTab(-1),
      moveTabRight: () => moveActiveTab(1),
      splitRight: () => splitActivePane('horizontal'),
      splitDown: () => splitActivePane('vertical'),
      closePane: () => {
        const st = useSessionStore.getState()
        const t = st.tabs.find((tab) => tab.id === st.activeTabId)
        if (t?.kind === 'panes') void requestClosePane(t.id, t.activePaneId)
      },
      zoomIn: () => nudgeZoom(1),
      zoomOut: () => nudgeZoom(-1),
      zoomReset: () => nudgeZoom(0, true),
      broadcast: () => {
        const st = useSessionStore.getState()
        const t = st.tabs.find((tab) => tab.id === st.activeTabId)
        if (t?.kind === 'panes') st.toggleBroadcast(t.id)
      },
    },
    settingsHotkeys,
  )

  useEffect(() => {
    loadHosts().catch(console.error)
    loadShells().catch(console.error)
  }, [loadHosts, loadShells])
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
    clearEditTemp().catch(console.error)
  }, [])
  // Closing the window stops every tab, so a tag that asks before closing asks here too.
  useEffect(() => {
    let allowed = false
    let un: Promise<() => void> | null = null
    try {
      un = getCurrentWindow().onCloseRequested(async (event) => {
        if (allowed) return
        const names = hostsAskingToClose()
        if (!names) return
        event.preventDefault()
        const ok = await confirm({
          title: 'Close Wisp?',
          body: `Tabs of ${names} are tagged to ask first. Anything still running in them stops.`,
          confirmLabel: 'Close Wisp',
          destructive: true,
        })
        if (!ok) return
        allowed = true
        await getCurrentWindow().close()
      })
    } catch {
      // not running inside a Tauri window (tests / web preview)
    }
    return () => {
      un?.then((f) => f())
    }
  }, [])
  useEffect(() => {
    loadSettings().catch(console.error)
  }, [loadSettings])
  useEffect(() => {
    // Following the system, a switch to dark picks the dark theme; themes can change too.
    const reapply = () => applyAppearance(useSettingsStore.getState().settings)
    const unwatch = watchSystemTheme(reapply)
    const unsubscribe = useThemeStore.subscribe(reapply)
    useThemeStore.getState().load().catch(console.error)
    return () => {
      unwatch()
      unsubscribe()
    }
  }, [])
  useEffect(() => {
    const un = events.tunnelStatus.listen((e) => {
      const { tunnelId, state, message } = e.payload
      useTunnelStore.getState().setStatus({ ...e.payload, state: state as TunnelState })
      // A stop the user asked for carries no reason; one the connection caused does.
      if (state !== 'stopped' || !message) return
      const tunnel = useHostStore
        .getState()
        .hosts.flatMap((h) => h.connections)
        .flatMap((c) => (c.kind.type === 'ssh' ? (c.kind.tunnels ?? []) : []))
        .find((t) => t.id === tunnelId)
      if (tunnel) tunnelNotice(tunnel, message)
    })
    // A background connection has no pane to clear up after it.
    const unSsh = events.sshStatus.listen((e) => {
      if (e.payload.state === 'disconnected') {
        useTunnelStore.getState().clearSession(e.payload.sessionId)
      }
    })
    return () => {
      un.then((f) => f())
      unSsh.then((f) => f())
    }
  }, [])

  const exportToFile = () => openPage('export')

  const activeTab = tabs.find((t) => t.id === activeTabId)
  // The window wears the colour of what the active tab shows.
  const frame = activeTab && tagColor(tabRules(activeTab, panes, hostsList, tagList).color)?.ring
  // In a fixed order: reordering tabs would move a view's element, and a moved element loses
  // keyboard focus.
  const placed = [...tabs].sort((a, b) => a.id.localeCompare(b.id))
  const panesTabs = placed.filter((t): t is PanesTab => t.kind === 'panes')
  const vncTabs = placed.filter((t): t is VncTab => t.kind === 'vnc')
  const rdpTabs = placed.filter((t): t is RdpTab => t.kind === 'rdp')
  return (
    <>
      <VaultGate onReady={() => setVaultReady(true)} />
      <Notices />
      <ConfirmDialog />
      {tunnelStarter.dialog}
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onStartTunnel={tunnelStarter.start}
        actions={[
          { id: 'ssh-once', label: 'Connect to SSH once…', run: () => setSshDialogOpen(true) },
          { id: 'sftp-once', label: 'Connect to SFTP once…', run: () => setSftpDialogOpen(true) },
          {
            id: 'ftp-once',
            label: 'Connect to FTP or FTPS once…',
            run: () => setFtpDialogOpen(true),
          },
          { id: 'vnc-once', label: 'Connect to VNC once…', run: () => setVncDialogOpen(true) },
          { id: 'rdp-once', label: 'Connect to RDP once…', run: () => setRdpDialogOpen(true) },
          {
            id: 'ssh-import',
            label: 'Import from ~/.ssh/config',
            run: () => openPage('sshImport'),
          },
          { id: 'import', label: 'Import a Wisp export…', run: () => void importFromFile() },
          { id: 'export', label: 'Export hosts…', run: exportToFile },
        ]}
      />
      <VncConnectDialog
        open={vncDialogOpen || once?.type === 'vnc'}
        onOpenChange={closing(setVncDialogOpen)}
        initial={onceFor('vnc')}
        onConnect={async (host, port, username, password) => {
          let secretId: string | null = null
          if (password) {
            try {
              secretId = await setSecret(password)
            } catch (e) {
              notify({ title: 'Could not store the password', body: String(e), tone: 'error' })
              return
            }
          }
          openVnc({ once: { host, port, username: username.trim() || null, secretId } })
        }}
      />
      <RdpConnectDialog
        open={rdpDialogOpen || once?.type === 'rdp'}
        onOpenChange={closing(setRdpDialogOpen)}
        initial={onceFor('rdp')}
        onConnect={async (host, port, username, domain, password) => {
          let secretId: string | null = null
          if (password) {
            try {
              secretId = await setSecret(password)
            } catch (e) {
              notify({ title: 'Could not store the password', body: String(e), tone: 'error' })
              return
            }
          }
          openRdp({ once: { host, port, username, domain, secretId, keyboardLayout: null } })
        }}
      />
      <FtpConnectDialog
        open={ftpDialogOpen || once?.type === 'ftp' || once?.type === 'ftps'}
        onOpenChange={closing(setFtpDialogOpen)}
        initial={onceFor('ftp', 'ftps')}
        onConnect={async ({ password, ...rest }) =>
          openFtp({ ...rest, secretId: password ? await setSecret(password) : null })
        }
      />
      <SftpConnectDialog
        open={sftpDialogOpen || once?.type === 'sftp'}
        onOpenChange={closing(setSftpDialogOpen)}
        initial={onceFor('sftp')}
        onPick={openSftp}
        onConnect={async (form) => openSftpAdhoc(await toAdhocParams(form))}
      />
      <SshConnectDialog
        open={sshDialogOpen || once?.type === 'ssh'}
        onOpenChange={closing(setSshDialogOpen)}
        initial={onceFor('ssh')}
        onConnect={async (form) => {
          try {
            openSshAdhoc(await toAdhocParams(form))
          } catch (e) {
            notify({ title: 'Could not store the password', body: String(e), tone: 'error' })
          }
        }}
      />
      <AppShell
        main={
          <div className={cn('flex h-full flex-col', frame && cn('ring-2 ring-inset', frame))}>
            <TabBar titleBar={customTitleBar} onPalette={() => setPaletteOpen(true)} />
            <div className={cn('flex min-h-0 flex-1', hostListRight && 'flex-row-reverse')}>
              {activeTabId !== null && <SessionHostList />}
              <div className="relative min-w-0 flex-1">
                <div
                  data-testid="tabpane-hosts"
                  className={cn('absolute inset-0', activeTabId !== null && 'hidden')}
                >
                  <HostsTab />
                </div>
                {panesTabs.map((t) => (
                  <div
                    key={t.id}
                    data-testid={`tabpane-${t.id}`}
                    className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                  >
                    <PanesView tab={t} />
                  </div>
                ))}
                <PaneLayer />
                {vncTabs.map((t) => (
                  <div
                    key={t.id}
                    data-testid={`tabpane-${t.id}`}
                    className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                  >
                    <VncTabView tab={t} active={t.id === activeTabId} />
                  </div>
                ))}
                {rdpTabs.map((t) => (
                  <div
                    key={t.id}
                    data-testid={`tabpane-${t.id}`}
                    className={cn('absolute inset-0', t.id !== activeTabId && 'hidden')}
                  >
                    <RdpTabView tab={t} active={t.id === activeTabId} />
                  </div>
                ))}
              </div>
            </div>
          </div>
        }
      />
    </>
  )
}
