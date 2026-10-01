import { useState } from 'react'
import type { ExportSummary, Group, Profile } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import { exportBundle, pickExportPath } from '@/lib/transfer'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import type { Tri } from './review'
import { TriCheckbox } from './TriCheckbox'

function descendants(groupId: string, groups: Group[], profiles: Profile[]): string[] {
  const keys = [`group:${groupId}`]
  for (const p of profiles) if (p.groupId === groupId) keys.push(`ssh:${p.id}`)
  for (const g of groups)
    if (g.parentId === groupId) keys.push(...descendants(g.id, groups, profiles))
  return keys
}

function stateOf(keys: string[], selected: Set<string>): Tri {
  const on = keys.filter((k) => selected.has(k)).length
  if (on === keys.length) return 'all'
  return on === 0 ? 'none' : 'some'
}

export function ExportPage({ tabId }: { tabId: string }) {
  const groups = useProfileStore((s) => s.groups)
  const profiles = useProfileStore((s) => s.profiles)
  const sftp = useSftpProfileStore((s) => s.profiles)
  const s3 = useS3ProfileStore((s) => s.profiles)
  const removeTab = useSessionStore((s) => s.removeTab)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [includeSecrets, setIncludeSecrets] = useState(false)
  const [includeKeys, setIncludeKeys] = useState(false)
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [summary, setSummary] = useState<ExportSummary | null>(null)

  const toggle = (keys: string[], on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev)
      for (const k of keys) {
        if (on) next.add(k)
        else next.delete(k)
      }
      return next
    })

  const needsPassword = includeSecrets || includeKeys
  const passwordOk = !needsPassword || (password.length > 0 && password === confirm)
  const ids = (prefix: string) =>
    [...selected].filter((k) => k.startsWith(prefix)).map((k) => k.slice(prefix.length))

  async function doExport() {
    const path = await pickExportPath()
    if (!path) return
    setBusy(true)
    setError(null)
    try {
      setSummary(
        await exportBundle(
          {
            groupIds: ids('group:'),
            profileIds: ids('ssh:'),
            sftpIds: ids('sftp:'),
            s3Ids: ids('s3:'),
          },
          { includeSecrets, includeKeys },
          needsPassword ? password : null,
          path,
        ),
      )
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const renderGroup = (g: Group) => {
    const keys = descendants(g.id, groups, profiles)
    return (
      <li key={g.id}>
        <label htmlFor={`export-group-${g.id}`} className="flex items-center gap-2">
          <TriCheckbox
            id={`export-group-${g.id}`}
            state={stateOf(keys, selected)}
            label={g.name}
            onChange={(on) => toggle(keys, on)}
          />
          <span className="font-medium">{g.name}</span>
        </label>
        <ul className="space-y-1 pl-6 pt-1">
          {groups.filter((c) => c.parentId === g.id).map(renderGroup)}
          {profiles.filter((p) => p.groupId === g.id).map(renderProfile)}
        </ul>
      </li>
    )
  }

  const renderProfile = (p: Profile) => (
    <li key={p.id}>
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          aria-label={p.name}
          checked={selected.has(`ssh:${p.id}`)}
          onChange={(e) => toggle([`ssh:${p.id}`], e.target.checked)}
        />
        {p.name}
      </label>
    </li>
  )

  const flat = (prefix: string, items: { id: string; name: string }[]) =>
    items.map((p) => (
      <li key={p.id}>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            aria-label={p.name}
            checked={selected.has(`${prefix}${p.id}`)}
            onChange={(e) => toggle([`${prefix}${p.id}`], e.target.checked)}
          />
          {p.name}
        </label>
      </li>
    ))

  const everything = [
    ...groups.map((g) => `group:${g.id}`),
    ...profiles.map((p) => `ssh:${p.id}`),
    ...sftp.map((p) => `sftp:${p.id}`),
    ...s3.map((p) => `s3:${p.id}`),
  ]

  if (summary) {
    return (
      <PageShell
        title="Export profiles"
        footer={
          <Button type="button" onClick={() => removeTab(tabId)}>
            Close
          </Button>
        }
      >
        <p className="text-sm">
          Exported {summary.profiles} profiles ({summary.secrets} passwords, {summary.keyFiles} key
          files).
        </p>
        <ul className="mt-2 space-y-1 text-muted-foreground text-sm">
          {summary.warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      </PageShell>
    )
  }

  return (
    <PageShell
      title="Export profiles"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={busy || selected.size === 0 || !passwordOk}
            onClick={doExport}
          >
            Export
          </Button>
        </>
      }
    >
      <div className="space-y-4 text-sm">
        <div className="flex gap-2">
          <Button type="button" variant="ghost" onClick={() => toggle(everything, true)}>
            Select all
          </Button>
          <Button type="button" variant="ghost" onClick={() => setSelected(new Set())}>
            Select none
          </Button>
        </div>
        <section>
          <h3 className="mb-1 font-semibold">SSH</h3>
          <ul className="space-y-1">
            {groups.filter((g) => !g.parentId).map(renderGroup)}
            {profiles.filter((p) => !p.groupId).map(renderProfile)}
          </ul>
        </section>
        {sftp.length > 0 && (
          <section>
            <h3 className="mb-1 font-semibold">SFTP</h3>
            <ul className="space-y-1">{flat('sftp:', sftp)}</ul>
          </section>
        )}
        {s3.length > 0 && (
          <section>
            <h3 className="mb-1 font-semibold">S3</h3>
            <ul className="space-y-1">{flat('s3:', s3)}</ul>
          </section>
        )}
        <section className="space-y-2 border-border border-t pt-4">
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              aria-label="Include passwords and passphrases"
              checked={includeSecrets}
              onChange={(e) => setIncludeSecrets(e.target.checked)}
            />
            Include passwords and passphrases
          </label>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              aria-label="Include private key files"
              checked={includeKeys}
              onChange={(e) => setIncludeKeys(e.target.checked)}
            />
            Include private key files
          </label>
          {needsPassword && (
            <div className="space-y-2">
              <p className="text-muted-foreground">
                The file is encrypted with this password. You'll need it to import.
              </p>
              <div className="space-y-1">
                <Label htmlFor="export-password">Export password</Label>
                <Input
                  id="export-password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="export-confirm">Confirm password</Label>
                <Input
                  id="export-confirm"
                  type="password"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                />
              </div>
            </div>
          )}
        </section>
        {error && <p className="text-destructive">{error}</p>}
      </div>
    </PageShell>
  )
}
