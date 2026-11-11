import { useState } from 'react'
import type { ExportSummary, Folder, Host } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import { THIS_COMPUTER } from '@/lib/hosts'
import { exportBundle, pickExportPath } from '@/lib/transfer'
import { useHostStore } from '@/stores/hostStore'
import type { Tri } from './review'
import { TriCheckbox } from './TriCheckbox'

// The hosts in a folder and all its subfolders.
function hostsUnder(folderId: string, folders: Folder[], hosts: Host[]): string[] {
  const ids = hosts.filter((h) => h.folderId === folderId).map((h) => h.id)
  for (const f of folders)
    if (f.parentId === folderId) ids.push(...hostsUnder(f.id, folders, hosts))
  return ids
}

function stateOf(ids: string[], selected: Set<string>): Tri {
  const on = ids.filter((id) => selected.has(id)).length
  if (on > 0 && on === ids.length) return 'all'
  return on === 0 ? 'none' : 'some'
}

function plural(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}

export function ExportPage({ onClose }: { onClose: () => void }) {
  const folders = useHostStore((s) => s.folders)
  const allHosts = useHostStore((s) => s.hosts)
  const hosts = allHosts.filter((h) => h.id !== THIS_COMPUTER)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [includeSecrets, setIncludeSecrets] = useState(false)
  const [includeKeys, setIncludeKeys] = useState(false)
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [summary, setSummary] = useState<ExportSummary | null>(null)

  const toggle = (ids: string[], on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev)
      for (const id of ids) {
        if (on) next.add(id)
        else next.delete(id)
      }
      return next
    })

  const needsPassword = includeSecrets || includeKeys
  const passwordOk = !needsPassword || (password.length > 0 && password === confirm)
  const byOrder = <T extends { order: number }>(items: T[]) =>
    [...items].sort((a, b) => a.order - b.order)

  async function doExport() {
    setBusy(true)
    setError(null)
    try {
      const path = await pickExportPath()
      if (!path) return
      const folderIds = folders
        .filter((f) => stateOf(hostsUnder(f.id, folders, hosts), selected) === 'all')
        .map((f) => f.id)
      const hostIds = hosts.filter((h) => selected.has(h.id)).map((h) => h.id)
      setSummary(
        await exportBundle(
          { folderIds, hostIds },
          { includeSecrets, includeKeys },
          needsPassword ? password : null,
          path,
        ),
      )
      setPassword('')
      setConfirm('')
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const renderHost = (h: Host) => (
    <li key={h.id}>
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          aria-label={h.name}
          checked={selected.has(h.id)}
          onChange={(e) => toggle([h.id], e.target.checked)}
        />
        {h.name}
      </label>
    </li>
  )

  const renderFolder = (f: Folder) => {
    const ids = hostsUnder(f.id, folders, hosts)
    return (
      <li key={f.id}>
        <label htmlFor={`export-folder-${f.id}`} className="flex items-center gap-2">
          <TriCheckbox
            id={`export-folder-${f.id}`}
            state={stateOf(ids, selected)}
            label={f.name}
            onChange={(on) => toggle(ids, on)}
          />
          <span className="font-medium">{f.name}</span>
        </label>
        <ul className="space-y-1 pt-1 pl-6">
          {byOrder(folders.filter((c) => c.parentId === f.id)).map(renderFolder)}
          {byOrder(hosts.filter((h) => h.folderId === f.id)).map(renderHost)}
        </ul>
      </li>
    )
  }

  if (summary) {
    return (
      <PageShell
        title="Export hosts"
        footer={
          <Button type="button" onClick={onClose}>
            Close
          </Button>
        }
      >
        <p className="text-sm">
          Exported {plural(summary.hosts, 'host')} with {plural(summary.connections, 'connection')}{' '}
          ({plural(summary.secrets, 'password')}, {plural(summary.keyFiles, 'key file')}).
        </p>
        <ul className="mt-2 space-y-1 text-muted-foreground text-sm">
          {[...new Set(summary.warnings)].map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      </PageShell>
    )
  }

  const known = new Set(folders.map((f) => f.id))
  return (
    <PageShell
      title="Export hosts"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>
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
          <Button
            type="button"
            variant="ghost"
            onClick={() =>
              toggle(
                hosts.map((h) => h.id),
                true,
              )
            }
          >
            Select all
          </Button>
          <Button type="button" variant="ghost" onClick={() => setSelected(new Set())}>
            Select none
          </Button>
        </div>
        <ul className="space-y-1">
          {byOrder(folders.filter((f) => !f.parentId || !known.has(f.parentId))).map(renderFolder)}
          {byOrder(hosts.filter((h) => !h.folderId || !known.has(h.folderId))).map(renderHost)}
        </ul>
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
                {confirm.length > 0 && confirm !== password && (
                  <p className="text-destructive">Passwords don't match.</p>
                )}
              </div>
            </div>
          )}
        </section>
        {error && <p className="text-destructive">{error}</p>}
      </div>
    </PageShell>
  )
}
