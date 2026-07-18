import { openUrl } from '@tauri-apps/plugin-opener'
import { RotateCcw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { COLOR_SCHEMES } from '@/features/sessions/terminalTheme'
import {
  chordFor,
  chordOf,
  conflictFor,
  formatChord,
  HOTKEY_ACTIONS,
  type HotkeyAction,
  suspendHotkeys,
} from '@/lib/hotkeys'
import { ACCENTS, BACKGROUNDS } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { vaultChangePassword } from '@/lib/vault'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { TerminalPreview } from './TerminalPreview'

const SECTIONS = ['Appearance', 'Keyboard', 'Transfers', 'Security', 'About'] as const
const REPO_URL = 'https://github.com/headpatcloud/wisp'
type Section = (typeof SECTIONS)[number]

export function SettingsPage({ tabId }: { tabId: string }) {
  const settings = useSettingsStore((s) => s.settings)
  const update = useSettingsStore((s) => s.update)
  const removeTab = useSessionStore((s) => s.removeTab)
  const [section, setSection] = useState<Section>('Appearance')
  const [masterPw, setMasterPw] = useState('')
  const [pwSaved, setPwSaved] = useState(false)
  const [capturing, setCapturing] = useState<HotkeyAction | null>(null)
  const [bindError, setBindError] = useState<string | null>(null)

  useEffect(() => {
    if (!capturing) return
    suspendHotkeys(true)
    setBindError(null)
    const onKey = (ev: KeyboardEvent) => {
      if (['Control', 'Shift', 'Alt', 'Meta'].includes(ev.key)) return
      ev.preventDefault()
      ev.stopPropagation()
      if (ev.key === 'Escape') {
        setCapturing(null)
        return
      }
      // Without a modifier the key would be swallowed before the terminal ever sees it.
      if (!ev.ctrlKey && !ev.altKey && !ev.metaKey) {
        setBindError('Shortcuts need Ctrl, Alt or Cmd.')
        return
      }
      const chord = chordOf(ev)
      const clash = conflictFor(capturing, chord, settings.hotkeys ?? {})
      if (clash) {
        setBindError(
          `${formatChord(chord)} is already used by "${HOTKEY_ACTIONS.find((a) => a.id === clash)?.label}".`,
        )
        return
      }
      update({ hotkeys: { ...(settings.hotkeys ?? {}), [capturing]: chord } })
      setCapturing(null)
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      suspendHotkeys(false)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [capturing, settings.hotkeys, update])

  return (
    <PageShell
      title="Settings"
      footer={
        <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
          Close
        </Button>
      }
    >
      <div className="flex gap-4">
        <nav className="flex w-40 shrink-0 flex-col gap-1">
          {SECTIONS.map((s) => (
            <button
              key={s}
              type="button"
              onClick={() => setSection(s)}
              className={cn(
                'rounded px-2 py-1 text-left text-sm hover:bg-muted',
                section === s && 'bg-muted font-medium',
              )}
            >
              {s}
            </button>
          ))}
        </nav>
        <div className="min-w-0 flex-1 space-y-4">
          {section === 'Appearance' && (
            <>
              <h3 className="font-medium text-sm">Application</h3>
              <div className="space-y-1">
                <Label htmlFor="theme">Theme</Label>
                <Select value={settings.theme} onValueChange={(v) => update({ theme: v })}>
                  <SelectTrigger id="theme">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="system">System</SelectItem>
                    <SelectItem value="light">Light</SelectItem>
                    <SelectItem value="dark">Dark</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <Label htmlFor="accent">Accent color</Label>
                <Select
                  value={settings.accent ?? 'teal'}
                  onValueChange={(v) => update({ accent: v })}
                >
                  <SelectTrigger id="accent">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ACCENTS.map((a) => (
                      <SelectItem key={a.value} value={a.value}>
                        <span className="flex items-center gap-2">
                          <span
                            className="size-3 rounded-full"
                            style={{ background: `oklch(0.66 0.13 ${a.hue})` }}
                          />
                          {a.label}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <Label htmlFor="background">Background color</Label>
                <Select
                  value={settings.background ?? 'teal'}
                  onValueChange={(v) => update({ background: v })}
                >
                  <SelectTrigger id="background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {BACKGROUNDS.map((b) => (
                      <SelectItem key={b.value} value={b.value}>
                        <span className="flex items-center gap-2">
                          <span
                            className="size-3 rounded-full border border-border"
                            style={{ background: `oklch(0.8 0.06 ${b.hue})` }}
                          />
                          {b.label}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-muted-foreground text-xs">
                  Tints the whole app (sidebar, panels, borders). The terminal uses its own color
                  scheme below.
                </p>
              </div>

              <h3 className="border-border border-t pt-4 font-medium text-sm">Terminal</h3>
              <TerminalPreview />
              <div className="space-y-1">
                <Label htmlFor="colorScheme">Color scheme</Label>
                <Select
                  value={settings.colorScheme}
                  onValueChange={(v) => update({ colorScheme: v })}
                >
                  <SelectTrigger id="colorScheme">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {COLOR_SCHEMES.map((s) => (
                      <SelectItem key={s.value} value={s.value}>
                        {s.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <Label htmlFor="fontFamily">Font family</Label>
                <Input
                  id="fontFamily"
                  value={settings.fontFamily}
                  onChange={(e) => update({ fontFamily: e.target.value })}
                />
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <Label htmlFor="fontSize">Font size</Label>
                  <Input
                    id="fontSize"
                    type="number"
                    value={settings.fontSize}
                    onChange={(e) => {
                      const n = Number(e.target.value)
                      if (Number.isFinite(n) && n > 0) update({ fontSize: n })
                    }}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="fontWeight">Font weight</Label>
                  <Select
                    value={settings.fontWeight ?? 'normal'}
                    onValueChange={(v) => update({ fontWeight: v })}
                  >
                    <SelectTrigger id="fontWeight">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="normal">Normal</SelectItem>
                      <SelectItem value="bold">Bold</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <Label htmlFor="lineHeight">Line height</Label>
                  <Input
                    id="lineHeight"
                    type="number"
                    step="0.1"
                    value={settings.lineHeight ?? 1}
                    onChange={(e) => {
                      const n = Number(e.target.value)
                      if (Number.isFinite(n) && n > 0) update({ lineHeight: n })
                    }}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="letterSpacing">Letter spacing</Label>
                  <Input
                    id="letterSpacing"
                    type="number"
                    step="0.5"
                    value={settings.letterSpacing ?? 0}
                    onChange={(e) => {
                      const n = Number(e.target.value)
                      if (Number.isFinite(n)) update({ letterSpacing: n })
                    }}
                  />
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="cursorStyle">Cursor style</Label>
                <Select
                  value={settings.cursorStyle ?? 'block'}
                  onValueChange={(v) => update({ cursorStyle: v })}
                >
                  <SelectTrigger id="cursorStyle">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="block">Block</SelectItem>
                    <SelectItem value="bar">Bar</SelectItem>
                    <SelectItem value="underline">Underline</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={settings.cursorBlink ?? true}
                  onChange={(e) => update({ cursorBlink: e.target.checked })}
                />
                Blink cursor
              </label>
              <div className="space-y-1">
                <Label htmlFor="scrollback">Scrollback (lines)</Label>
                <Input
                  id="scrollback"
                  type="number"
                  value={settings.scrollback ?? 10000}
                  onChange={(e) => {
                    const n = Number(e.target.value)
                    if (Number.isFinite(n) && n >= 0) update({ scrollback: n })
                  }}
                />
              </div>
              <div className="border-border border-t pt-3">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={settings.copyOnSelect ?? false}
                    onChange={(e) => update({ copyOnSelect: e.target.checked })}
                  />
                  Copy on select
                </label>
                <label className="mt-2 flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={settings.rightClickPaste ?? false}
                    onChange={(e) => update({ rightClickPaste: e.target.checked })}
                  />
                  Right-click pastes
                </label>
                <p className="mt-1 text-muted-foreground text-xs">
                  PuTTY-style mouse behaviour in terminals. Both off by default.
                </p>
                <label className="mt-3 flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={settings.restoreSession ?? true}
                    onChange={(e) => update({ restoreSession: e.target.checked })}
                  />
                  Reopen tabs on startup
                </label>
                <p className="mt-1 text-muted-foreground text-xs">
                  Restores SSH, SFTP and S3 tabs. FTP and VNC tabs are skipped because their
                  passwords would have to be stored on disk.
                </p>
              </div>
            </>
          )}
          {section === 'Keyboard' && (
            <div className="space-y-1">
              <p className="pb-2 text-muted-foreground text-xs">
                Click a shortcut to rebind it. Escape cancels.
              </p>
              {bindError && <p className="pb-2 text-destructive text-xs">{bindError}</p>}
              {HOTKEY_ACTIONS.map((a) => (
                <div key={a.id} className="flex items-center gap-2 py-1 text-sm">
                  <span className="min-w-0 flex-1 truncate">{a.label}</span>
                  <button
                    type="button"
                    onClick={() => setCapturing(a.id)}
                    className={cn(
                      'rounded border border-border px-2 py-1 font-mono text-xs hover:bg-muted',
                      capturing === a.id && 'border-ring text-muted-foreground',
                    )}
                  >
                    {capturing === a.id
                      ? 'Press keys…'
                      : formatChord(chordFor(a.id, settings.hotkeys ?? {}))}
                  </button>
                  {settings.hotkeys?.[a.id] && (
                    <button
                      type="button"
                      aria-label={`Reset ${a.label}`}
                      onClick={() => {
                        const next = { ...(settings.hotkeys ?? {}) }
                        delete next[a.id]
                        update({ hotkeys: next })
                      }}
                      className="rounded p-1 text-muted-foreground hover:bg-muted"
                    >
                      <RotateCcw className="size-3.5" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
          {section === 'Transfers' && (
            <div className="space-y-2">
              <div className="space-y-1">
                <Label htmlFor="maxTransfers">Concurrent transfers</Label>
                <Input
                  id="maxTransfers"
                  type="number"
                  min={1}
                  max={16}
                  value={settings.maxConcurrentTransfers ?? 3}
                  onChange={(e) => {
                    const n = Number(e.target.value)
                    if (Number.isFinite(n) && n >= 1) {
                      update({ maxConcurrentTransfers: Math.min(16, Math.floor(n)) })
                    }
                  }}
                />
                <p className="text-muted-foreground text-xs">
                  How many uploads or downloads run at once. The rest wait in a queue. 1-16, default
                  3.
                </p>
              </div>
            </div>
          )}
          {section === 'Security' && (
            <div className="space-y-2">
              <div className="space-y-1">
                <Label htmlFor="masterPw">Master password</Label>
                <Input
                  id="masterPw"
                  type="password"
                  value={masterPw}
                  onChange={(e) => {
                    setMasterPw(e.target.value)
                    setPwSaved(false)
                  }}
                />
                <p className="text-muted-foreground text-xs">
                  Encrypts stored secrets with this password instead of the OS keychain. You'll
                  enter it on each launch.
                </p>
              </div>
              <Button
                type="button"
                disabled={!masterPw}
                onClick={async () => {
                  await vaultChangePassword(masterPw)
                  setMasterPw('')
                  setPwSaved(true)
                }}
              >
                Set master password
              </Button>
              {pwSaved && <p className="text-xs">Master password updated.</p>}
              <div className="border-border border-t pt-3">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={settings.vncClipboardSync}
                    onChange={(e) => update({ vncClipboardSync: e.target.checked })}
                  />
                  Sync clipboard from VNC servers
                </label>
                <p className="mt-1 text-muted-foreground text-xs">
                  Lets a connected VNC server write to your local clipboard. Off by default.
                </p>
              </div>
            </div>
          )}
          {section === 'About' && (
            <div className="space-y-3">
              <div className="flex items-center gap-3">
                <img src="/wisp-icon.svg" alt="" className="size-12" />
                <div>
                  <h3 className="font-semibold text-lg">Wisp</h3>
                  <p className="text-muted-foreground text-sm">Version {__APP_VERSION__}</p>
                </div>
              </div>
              <p className="text-muted-foreground text-sm">
                A fast, secure desktop client for SSH, SFTP, FTP, and VNC, built with Tauri and
                React.
              </p>
              <button
                type="button"
                onClick={() => openUrl(REPO_URL).catch(() => {})}
                className="text-primary text-sm underline-offset-2 hover:underline"
              >
                {REPO_URL.replace('https://', '')}
              </button>
            </div>
          )}
        </div>
      </div>
    </PageShell>
  )
}
