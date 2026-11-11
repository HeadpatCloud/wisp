import { useEffect, useState } from 'react'
import { commands } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { checkForUpdates } from '@/features/updater/updateNotice'
import { unwrap } from '@/lib/ipc'
import { vaultChangePassword, vaultUnlock } from '@/lib/vault'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { Heading, Row, Toggle } from './controls'

const errorText = (e: unknown) =>
  e && typeof e === 'object' && 'message' in e ? String(e.message) : String(e)

function MasterPassword({ usesPassword, onDone }: { usesPassword: boolean; onDone: () => void }) {
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [again, setAgain] = useState('')
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    // The vault is open either way; asking for the current password keeps a passer-by from
    // locking the owner out.
    if (usesPassword) await vaultUnlock(current)
    await vaultChangePassword(next)
    onDone()
  }
  return (
    <div className="grid gap-3 rounded-lg border border-border p-3 sm:grid-cols-2">
      {usesPassword && (
        <Row id="pw-current" label="Current master password">
          <Input
            id="pw-current"
            type="password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
          />
        </Row>
      )}
      <Row id="pw-new" label="New master password">
        <Input id="pw-new" type="password" value={next} onChange={(e) => setNext(e.target.value)} />
      </Row>
      <Row id="pw-again" label="Type it again">
        <Input
          id="pw-again"
          type="password"
          value={again}
          onChange={(e) => setAgain(e.target.value)}
        />
      </Row>
      <p className="col-span-full text-muted-foreground text-xs">
        You'll type it each time Wisp starts. A forgotten master password can't be recovered.
      </p>
      {error && (
        <p role="alert" className="col-span-full text-destructive text-sm">
          {error}
        </p>
      )}
      <div className="col-span-full flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          type="button"
          disabled={!next || next !== again || (usesPassword && !current)}
          onClick={() => {
            setError(null)
            save().catch((e: unknown) =>
              setError(
                e && typeof e === 'object' && 'kind' in e && e.kind === 'wrongPassphrase'
                  ? 'The current master password is wrong.'
                  : errorText(e),
              ),
            )
          }}
        >
          Save
        </Button>
      </div>
    </div>
  )
}

export function GeneralPage() {
  const settings = useSettingsStore((s) => s.settings)
  const update = useSettingsStore((s) => s.update)
  const openPage = useHostsUiStore((s) => s.openPage)
  const [usesPassword, setUsesPassword] = useState<boolean | null>(null)
  const [editing, setEditing] = useState(false)

  const readVault = () => {
    commands
      .vaultUsesPassword()
      .then((r) => setUsesPassword(unwrap(r)))
      .catch(console.error)
  }
  useEffect(readVault, [])

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <Heading>Tabs</Heading>
      <Toggle
        label="Reopen the tabs from last time"
        checked={settings.restoreSession ?? true}
        onChange={(v) => update({ restoreSession: v })}
        hint="Passwords typed for a single connection stay in the vault."
      />

      <Heading>Transfers</Heading>
      <Row
        id="max-transfers"
        label="Transfers running at the same time"
        hint="The rest wait in a queue. 1 to 16."
      >
        <Input
          id="max-transfers"
          type="number"
          min={1}
          max={16}
          value={settings.maxConcurrentTransfers ?? 3}
          onChange={(e) => {
            const n = Number(e.target.value)
            if (Number.isFinite(n) && n >= 1)
              update({ maxConcurrentTransfers: Math.min(16, Math.floor(n)) })
          }}
        />
      </Row>

      <Heading>Remote desktops</Heading>
      <Toggle
        label="Share the clipboard with remote desktops"
        checked={settings.vncClipboardSync ?? false}
        onChange={(v) => update({ vncClipboardSync: v })}
        hint="Lets a connected VNC or RDP server read and write your clipboard. Off by default."
      />

      <Heading>Saved passwords</Heading>
      {usesPassword !== null && (
        <p className="text-sm">
          {usesPassword
            ? 'Passwords and passphrases are encrypted with your master password.'
            : 'Passwords and passphrases are encrypted with a key kept in the system keychain.'}
        </p>
      )}
      {editing && usesPassword !== null ? (
        <MasterPassword
          usesPassword={usesPassword}
          onDone={() => {
            setEditing(false)
            readVault()
          }}
        />
      ) : (
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" size="sm" onClick={() => setEditing(true)}>
            {usesPassword ? 'Change master password…' : 'Use a master password instead…'}
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={() => openPage('keys')}>
            Keys & passwords
          </Button>
        </div>
      )}

      <Heading>About</Heading>
      <div className="flex items-center gap-3">
        <img src="/wisp-icon.svg" alt="" className="size-10" />
        <p className="text-sm">Wisp {__APP_VERSION__}</p>
        <Button type="button" variant="outline" size="sm" onClick={() => void checkForUpdates()}>
          Check for updates
        </Button>
      </div>
    </div>
  )
}
