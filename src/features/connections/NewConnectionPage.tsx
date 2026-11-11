import { useEffect, useState } from 'react'
import type { Host } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import { addressKey } from '@/lib/address'
import { type ConnectionType, folderPath, THIS_COMPUTER, typeName } from '@/lib/hosts'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'
import { type NewConnectionPrefill, useHostsUiStore } from '@/stores/hostsUiStore'
import { openConnection } from '../hosts/openConnection'
import { ConnectionFields } from './ConnectionFields'
import { type ConnectionDraft, draftFrom, toConnection, validateDraft } from './drafts'
import { SelectField, TextField } from './fields'
import { HostColorField, TagsField } from './TagsField'

const TYPES: [ConnectionType, string, string][] = [
  ['ssh', 'SSH', 'Terminal'],
  ['sftp', 'Files (SFTP)', 'Over SSH'],
  ['ftp', 'FTP', 'FTP or FTPS'],
  ['s3', 'Storage (S3)', 'One bucket'],
  ['rdp', 'RDP', 'Windows desktop'],
  ['vnc', 'VNC', 'Screen sharing'],
  ['local', 'Local shell', 'This computer'],
]

const TITLES: Record<ConnectionType, string> = {
  ssh: 'SSH connection',
  sftp: 'Files over SFTP',
  ftp: 'FTP connection',
  s3: 'S3 bucket',
  rdp: 'Remote desktop (RDP)',
  vnc: 'Screen sharing (VNC)',
  local: 'Local shell',
}

const NO_FOLDER = 'none'
const REQUIRED = 'Fill this in.'

function joinAnd(items: string[]): string {
  if (items.length <= 1) return items.join('')
  return `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`
}

function nextOrder(hosts: Host[], folderId: string | null): number {
  const orders = hosts.filter((h) => h.folderId === folderId).map((h) => h.order + 1)
  return orders.length ? Math.max(...orders) : 0
}

export function NewConnectionPage({
  prefill,
  onClose,
}: {
  prefill: NewConnectionPrefill
  onClose: () => void
}) {
  const hosts = useHostStore((s) => s.hosts)
  const folders = useHostStore((s) => s.folders)
  const logins = useHostStore((s) => s.logins)
  const keys = useHostStore((s) => s.keys)
  const shells = useHostStore((s) => s.shells)
  const saveHost = useHostStore((s) => s.saveHost)
  const navFolder = useHostsUiStore((s) => s.folderId)
  const select = useHostsUiStore((s) => s.select)
  const data = { hosts, logins, keys }

  const fixedHost = prefill.hostId ? (hosts.find((h) => h.id === prefill.hostId) ?? null) : null
  const startType = prefill.type ?? (fixedHost?.id === THIS_COMPUTER ? 'local' : 'ssh')
  const [type, setType] = useState<ConnectionType>(startType)
  const [address, setAddress] = useState(prefill.address ?? fixedHost?.address ?? '')
  const [toExisting, setToExisting] = useState(true)
  const [name, setName] = useState<string | null>(null)
  const [folderId, setFolderId] = useState<string | null>(navFolder)
  const [tagIds, setTagIds] = useState<string[]>([])
  const [color, setColor] = useState<string | null>(null)
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [failure, setFailure] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const fresh = (t: ConnectionType, withPrefill: boolean): ConnectionDraft => {
    const d = draftFrom(t, null, fixedHost, data)
    if (!withPrefill) return d
    return {
      ...d,
      user: prefill.user ?? d.user,
      port: prefill.port ? String(prefill.port) : d.port,
      bucket: prefill.bucket ?? d.bucket,
    }
  }
  const [draft, setDraft] = useState<ConnectionDraft>(() => fresh(startType, true))

  const matchable = type !== 's3' && type !== 'local'
  const key = addressKey(address.trim())
  const matched =
    fixedHost ??
    (matchable && key
      ? (hosts.find((h) => h.id !== THIS_COMPUTER && addressKey(h.address) === key) ?? null)
      : null)
  const addTo = type === 'local' ? null : matched && (toExisting || fixedHost) ? matched : null
  const pc = hosts.find((h) => h.id === THIS_COMPUTER) ?? null
  const host = type === 'local' ? pc : addTo
  // Files uses the target host's SSH login by default, so it follows the host being added to.
  const targetHasSsh = !!host?.connections.some((c) => c.kind.type === 'ssh')
  useEffect(() => {
    if (type === 'sftp') setDraft((d) => ({ ...d, sameAsSsh: targetHasSsh }))
  }, [type, targetHasSsh])
  const defaultName = type === 's3' ? draft.bucket.trim() || address.trim() : address.trim()
  const hostName = name ?? defaultName

  const pickType = (t: ConnectionType) => {
    setType(t)
    setDraft(fresh(t, t === startType))
    setErrors({})
  }

  const save = async (connect: boolean) => {
    const found = validateDraft(draft)
    if (type !== 'local' && !address.trim()) found.address = REQUIRED
    if (type !== 'local' && !addTo && !hostName) found.name = REQUIRED
    setErrors(found)
    if (Object.keys(found).length > 0) return
    setSaving(true)
    setFailure(null)
    try {
      const conn = await toConnection(draft, data)
      let saved: Host
      if (host) {
        saved = { ...host, connections: [...host.connections, conn] }
      } else {
        saved = {
          id: crypto.randomUUID(),
          name: hostName,
          address: address.trim(),
          folderId,
          tagIds,
          icon: { kind: 'builtin', name: 'server' },
          color,
          order: nextOrder(hosts, folderId),
          connections: [conn],
        }
      }
      await saveHost(saved)
      select(saved.id)
      onClose()
      if (connect) openConnection(saved, conn)
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e))
      setSaving(false)
    }
  }

  const folderOptions: [string, string][] = [
    ...folders
      .map((f): [string, string] => [f.id, folderPath(f.id, folders)])
      .sort((a, b) => a[1].localeCompare(b[1])),
    [NO_FOLDER, 'No folder'],
  ]
  const where = (h: Host) => {
    const path = folderPath(h.folderId, folders)
    return path ? ` in ${path}` : ''
  }
  const typesOf = (h: Host) =>
    joinAnd([...new Set(h.connections.map((c) => typeName(c.kind.type)))])

  return (
    <PageShell
      title="New connection"
      footer={
        <>
          {failure && (
            <p role="alert" className="mr-auto self-center text-destructive text-sm">
              {failure}
            </p>
          )}
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="outline" disabled={saving} onClick={() => void save(false)}>
            Save
          </Button>
          <Button disabled={saving} onClick={() => void save(true)}>
            Save and connect
          </Button>
        </>
      }
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-6">
        <fieldset>
          <legend className="sr-only">Type</legend>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
            {TYPES.filter(
              ([t]) => !fixedHost || (fixedHost.id === THIS_COMPUTER) === (t === 'local'),
            ).map(([t, label, sub]) => (
              <button
                key={t}
                type="button"
                aria-pressed={type === t}
                onClick={() => pickType(t)}
                className={cn(
                  'flex flex-col rounded-lg border p-3 text-left',
                  type === t ? 'border-primary bg-accent' : 'border-border hover:bg-muted',
                )}
              >
                <span className="font-semibold text-sm">{label}</span>
                <span className="text-muted-foreground text-xs">{sub}</span>
              </button>
            ))}
          </div>
        </fieldset>

        {type !== 'local' && (
          <section className="flex flex-col gap-2">
            <Label htmlFor="new-address">{type === 's3' ? 'Endpoint' : 'Address'}</Label>
            <Input
              id="new-address"
              value={address}
              readOnly={!!fixedHost}
              aria-invalid={errors.address ? true : undefined}
              onChange={(e) => setAddress(e.target.value)}
            />
            {errors.address && <p className="text-destructive text-xs">{errors.address}</p>}
            {matched && !fixedHost && (
              <div className="flex flex-col gap-2 rounded-lg border border-border bg-muted p-3 text-sm">
                <p>
                  {matched.connections.length > 0
                    ? `${matched.name} is already a host${where(matched)}, with ${typesOf(matched)}.`
                    : `${matched.name} is already a host${where(matched)}.`}
                </p>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    variant={toExisting ? 'default' : 'outline'}
                    aria-pressed={toExisting}
                    onClick={() => setToExisting(true)}
                  >
                    Add to {matched.name}
                  </Button>
                  <Button
                    size="sm"
                    variant={toExisting ? 'outline' : 'default'}
                    aria-pressed={!toExisting}
                    onClick={() => setToExisting(false)}
                  >
                    Make a new host
                  </Button>
                </div>
              </div>
            )}
          </section>
        )}

        <section className="flex flex-col gap-3">
          <h2 className="font-semibold">{TITLES[type]}</h2>
          <ConnectionFields
            draft={draft}
            onChange={setDraft}
            errors={errors}
            host={host}
            file={data}
            shells={shells}
          />
        </section>

        {type !== 'local' &&
          (addTo ? (
            <p className="text-muted-foreground text-sm">
              Adds to {addTo.name}
              {where(addTo)}. Name, folder and tags stay as they are.
            </p>
          ) : (
            <section className="flex flex-col gap-3">
              <h2 className="font-semibold">New host</h2>
              <div className="grid gap-3 sm:grid-cols-2">
                <TextField
                  id="new-host-name"
                  label="Name"
                  value={hostName}
                  onChange={setName}
                  error={errors.name}
                />
                <SelectField
                  id="new-host-folder"
                  label="Folder"
                  value={folderId ?? NO_FOLDER}
                  onChange={(v) => setFolderId(v === NO_FOLDER ? null : v)}
                  options={folderOptions}
                />
                <TagsField value={tagIds} onChange={setTagIds} />
                <HostColorField value={color} onChange={setColor} />
              </div>
            </section>
          ))}
      </div>
    </PageShell>
  )
}
