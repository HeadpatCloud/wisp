import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { GripVertical, MoreHorizontal, Plus, Upload } from 'lucide-react'
import { useState } from 'react'
import type { Connection, ConnectionMove, IconRef, MoveTarget } from '@/bindings'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { PageShell } from '@/components/ui/page-shell'
import {
  type ConnectionType,
  connectionLabel,
  effectiveAddress,
  folderPath,
  THIS_COMPUTER,
  typeName,
} from '@/lib/hosts'
import { importIcon } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { confirm } from '@/stores/confirmStore'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { BUILTIN_ICONS } from '../hosts/icons'
import { ProfileIcon } from '../hosts/ProfileIcon'
import { usePointerDrag } from '../hosts/usePointerDrag'
import { ConnectionFields } from './ConnectionFields'
import { confirmDeleteHost, connectionDeleteText, deleteHostWithUndo, undoNotice } from './deletes'
import { type ConnectionDraft, draftFrom, toConnection, validateDraft } from './drafts'
import { SelectField, TextField } from './fields'
import { HostColorField, TagsField } from './TagsField'

interface Entry {
  key: string
  original: Connection | null
  draft: ConnectionDraft | null
  open: boolean
  errors: Record<string, string>
}

const REMOTE_TYPES: ConnectionType[] = ['ssh', 'sftp', 'ftp', 's3', 'rdp', 'vnc']
const NO_FOLDER = 'none'
const NEW_HOST = 'new'

export function HostEditorPage({
  hostId,
  openConnectionId = null,
  onClose,
}: {
  hostId: string
  openConnectionId?: string | null
  onClose: () => void
}) {
  const hosts = useHostStore((s) => s.hosts)
  const folders = useHostStore((s) => s.folders)
  const logins = useHostStore((s) => s.logins)
  const keys = useHostStore((s) => s.keys)
  const shells = useHostStore((s) => s.shells)
  const saveHost = useHostStore((s) => s.saveHost)
  const select = useHostsUiStore((s) => s.select)
  const host = hosts.find((h) => h.id === hostId)
  const data = { hosts, logins, keys }

  const [name, setName] = useState(host?.name ?? '')
  const [address, setAddress] = useState(host?.address ?? '')
  const [folderId, setFolderId] = useState<string | null>(host?.folderId ?? null)
  const [tagIds, setTagIds] = useState<string[]>(host?.tagIds ?? [])
  const [color, setColor] = useState<string | null>(host?.color ?? null)
  const [icon, setIcon] = useState<IconRef>(host?.icon ?? { kind: 'builtin', name: 'server' })
  const [entries, setEntries] = useState<Entry[]>(
    () =>
      host?.connections.map((c) => ({
        key: c.id,
        original: c,
        draft: c.id === openConnectionId ? draftFrom(c.kind.type, c, host, data) : null,
        open: c.id === openConnectionId,
        errors: {},
      })) ?? [],
  )
  const [moves, setMoves] = useState<{ entry: Entry; to: MoveTarget }[]>([])
  const [moving, setMoving] = useState<Entry | null>(null)
  const [moveTo, setMoveTo] = useState(NEW_HOST)
  const [newName, setNewName] = useState('')
  const [newAddress, setNewAddress] = useState('')
  const [nameError, setNameError] = useState<string | undefined>()
  const [failure, setFailure] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const reorder = (from: number, to: number) => {
    if (to < 0 || to >= entries.length || to === from) return
    const next = [...entries]
    const [moved] = next.splice(from, 1)
    next.splice(to, 0, moved)
    setEntries(next)
  }
  const drag = usePointerDrag<number>((from, target) => {
    if (target.kind === 'conn') reorder(from, target.index)
  })

  if (!host) {
    return (
      <PageShell title="Edit host" footer={<Button onClick={onClose}>Back</Button>}>
        <p className="text-sm">This host no longer exists.</p>
      </PageShell>
    )
  }
  const pc = host.id === THIS_COMPUTER
  const others = hosts.filter((h) => h.id !== host.id && h.id !== THIS_COMPUTER)

  const titleOf = (e: Entry) => {
    if (e.original) return connectionLabel(host, e.original, logins, shells)
    const type = typeName(e.draft?.type ?? 'ssh')
    const label = e.draft?.label.trim()
    return label ? `${type} · ${label}` : `New ${type} connection`
  }
  const update = (key: string, patch: Partial<Entry>) =>
    setEntries((list) => list.map((e) => (e.key === key ? { ...e, ...patch } : e)))
  const draftOf = (e: Entry) =>
    e.draft ?? draftFrom((e.original as Connection).kind.type, e.original, host, data)

  const add = (type: ConnectionType) => {
    const draft = draftFrom(type, null, host, data)
    setEntries([...entries, { key: draft.id, original: null, draft, open: true, errors: {} }])
  }

  const duplicate = (i: number) => {
    const e = entries[i]
    const base = draftOf(e)
    // What the original is called without its type: its label, else the shell or the user.
    const named =
      base.label.trim() ||
      (base.type === 'local'
        ? (shells.find((sh) => sh.program === base.program)?.name ?? base.program)
        : base.type === 'ssh' || base.type === 'rdp'
          ? base.user.trim()
          : '')
    const draft: ConnectionDraft = {
      ...base,
      id: crypto.randomUUID(),
      label: named ? `${named} (copy)` : 'copy',
      tunnels: base.tunnels.map((t) => ({ ...t, id: crypto.randomUUID() })),
    }
    const next = [...entries]
    next.splice(i + 1, 0, { key: draft.id, original: null, draft, open: true, errors: {} })
    setEntries(next)
  }

  const remove = async (e: Entry) => {
    if (e.original) {
      const text = connectionDeleteText(host, e.original, hosts, logins)
      if (!(await confirm({ ...text, confirmLabel: 'Delete', destructive: true }))) return
    }
    setEntries((list) => list.filter((x) => x.key !== e.key))
  }

  const openMove = (e: Entry) => {
    setMoving(e)
    setMoveTo(others[0]?.id ?? NEW_HOST)
    setNewName('')
    setNewAddress(e.original ? effectiveAddress(host, e.original) : host.address)
  }
  const move = () => {
    if (!moving) return
    const to: MoveTarget =
      moveTo === NEW_HOST
        ? {
            kind: 'newHost',
            name: newName.trim(),
            address: newAddress.trim(),
            folderId: host.folderId,
          }
        : { kind: 'host', hostId: moveTo }
    setMoves([...moves, { entry: moving, to }])
    setEntries((list) => list.filter((x) => x.key !== moving.key))
    setMoving(null)
  }

  const save = async () => {
    let bad = false
    const checked = entries.map((e) => {
      if (!e.draft) return e
      const errors = validateDraft(e.draft)
      if (Object.keys(errors).length === 0) return { ...e, errors }
      bad = true
      return { ...e, errors, open: true }
    })
    setEntries(checked)
    if (!pc && !name.trim()) {
      setNameError('Fill this in.')
      bad = true
    }
    if (bad) return
    setSaving(true)
    setFailure(null)
    try {
      const connections: Connection[] = []
      for (const e of checked) {
        connections.push(e.draft ? await toConnection(e.draft, data) : (e.original as Connection))
      }
      const moveList: ConnectionMove[] = []
      for (const m of moves) {
        const connection = m.entry.draft
          ? await toConnection(m.entry.draft, data)
          : (m.entry.original as Connection)
        moveList.push({ connection, to: m.to })
      }
      const gone = new Set([...connections, ...moveList.map((m) => m.connection)].map((c) => c.id))
      const removed = host.connections.filter((c) => !gone.has(c.id))
      const pending = await saveHost(
        {
          ...host,
          name: name.trim(),
          address: address.trim(),
          folderId,
          icon,
          tagIds,
          color,
          connections,
        },
        moveList,
      )
      pending.forEach((id, i) => {
        const c = removed[i]
        undoNotice(`${c ? connectionLabel(host, c, logins, shells) : 'Connection'} deleted`, id)
      })
      select(host.id)
      onClose()
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e))
      setSaving(false)
    }
  }

  const deleteHost = async () => {
    if (!(await confirmDeleteHost(host))) return
    try {
      await deleteHostWithUndo(host)
      onClose()
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <PageShell
      title={
        <>
          <span className="flex-1">Edit {host.name}</span>
          {!pc && (
            <Button variant="ghost" size="sm" onClick={() => void deleteHost()}>
              Delete host…
            </Button>
          )}
        </>
      }
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
          <Button disabled={saving} onClick={() => void save()}>
            Save
          </Button>
        </>
      }
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-6">
        {!pc && (
          <section className="flex flex-col gap-3">
            <fieldset className="flex flex-col gap-2">
              <legend className="mb-1 font-medium text-sm">Icon</legend>
              <div className="flex flex-wrap gap-1">
                {BUILTIN_ICONS.map(({ name: iconName, Icon }) => (
                  <button
                    key={iconName}
                    type="button"
                    aria-label={iconName}
                    aria-pressed={icon.kind === 'builtin' && icon.name === iconName}
                    onClick={() => setIcon({ kind: 'builtin', name: iconName })}
                    className={cn(
                      'rounded-md p-2 hover:bg-muted',
                      icon.kind === 'builtin' &&
                        icon.name === iconName &&
                        'bg-muted ring-1 ring-primary',
                    )}
                  >
                    <Icon className="size-4" aria-hidden />
                  </button>
                ))}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-pressed={icon.kind === 'custom'}
                  onClick={async () => {
                    const file = await pickFile({
                      filters: [
                        { name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'svg', 'webp', 'gif'] },
                      ],
                    })
                    if (typeof file === 'string')
                      setIcon({ kind: 'custom', path: await importIcon(file) })
                  }}
                >
                  {icon.kind === 'custom' ? (
                    <ProfileIcon icon={icon} className="size-4" />
                  ) : (
                    <Upload aria-hidden />
                  )}
                  Use an image…
                </Button>
              </div>
            </fieldset>
            <div className="grid gap-3 sm:grid-cols-2">
              <TextField
                id="edit-name"
                label="Name"
                value={name}
                onChange={(v) => {
                  setName(v)
                  setNameError(undefined)
                }}
                error={nameError}
              />
              <TextField id="edit-address" label="Address" value={address} onChange={setAddress} />
              <SelectField
                id="edit-folder"
                label="Folder"
                value={folderId ?? NO_FOLDER}
                onChange={(v) => setFolderId(v === NO_FOLDER ? null : v)}
                options={[
                  ...folders
                    .map((f): [string, string] => [f.id, folderPath(f.id, folders)])
                    .sort((a, b) => a[1].localeCompare(b[1])),
                  [NO_FOLDER, 'No folder'],
                ]}
              />
              <TagsField value={tagIds} onChange={setTagIds} />
              <HostColorField value={color} onChange={setColor} />
            </div>
          </section>
        )}

        <section className="flex flex-col gap-2">
          <h2 className="font-semibold">Connections</h2>
          <p className="text-muted-foreground text-sm">
            Drag to reorder. The first opens with Enter or a double-click.
          </p>
          <ul className="flex flex-col gap-2">
            {entries.map((e, i) => {
              const title = titleOf(e)
              return (
                <li
                  key={e.key}
                  data-drop-conn={i}
                  className="rounded-lg border border-border bg-background data-drop-over:ring-2 data-drop-over:ring-ring"
                >
                  <div className="flex items-center gap-2 p-2">
                    <button
                      type="button"
                      aria-label={`Drag to reorder ${title}`}
                      onPointerDown={(ev) => drag.start(i, ev)}
                      onKeyDown={(ev) => {
                        if (!ev.altKey) return
                        if (ev.key === 'ArrowUp') reorder(i, i - 1)
                        else if (ev.key === 'ArrowDown') reorder(i, i + 1)
                        else return
                        ev.preventDefault()
                      }}
                      className="cursor-grab rounded p-0.5 text-muted-foreground hover:bg-muted"
                    >
                      <GripVertical className="size-4" aria-hidden />
                    </button>
                    <p className="flex min-w-0 flex-1 items-center gap-1.5 truncate font-medium text-sm">
                      {title}
                      {i === 0 && (
                        <span className="rounded bg-muted px-1.5 font-medium text-muted-foreground text-xs">
                          Main
                        </span>
                      )}
                    </p>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-expanded={e.open}
                      onClick={() => update(e.key, { open: !e.open, draft: draftOf(e) })}
                    >
                      {e.open ? 'Done' : 'Edit'}
                    </Button>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <button
                          type="button"
                          aria-label={`More actions for ${title}`}
                          className="rounded p-1 text-muted-foreground hover:bg-muted"
                        >
                          <MoreHorizontal className="size-4" aria-hidden />
                        </button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem disabled={i === 0} onSelect={() => reorder(i, 0)}>
                          Make it the main connection
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => duplicate(i)}>Duplicate</DropdownMenuItem>
                        {!pc && (
                          <DropdownMenuItem onSelect={() => openMove(e)}>
                            Move to another host…
                          </DropdownMenuItem>
                        )}
                        <DropdownMenuItem onSelect={() => void remove(e)}>Delete…</DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  {e.open && e.draft && (
                    <div className="border-border border-t p-3">
                      <ConnectionFields
                        draft={e.draft}
                        onChange={(draft) => update(e.key, { draft })}
                        errors={e.errors}
                        host={host}
                        file={data}
                        shells={shells}
                      />
                    </div>
                  )}
                </li>
              )
            })}
          </ul>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="self-start">
                <Plus aria-hidden />
                Add a connection
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {(pc ? (['local'] as ConnectionType[]) : REMOTE_TYPES).map((t) => (
                <DropdownMenuItem key={t} onSelect={() => add(t)}>
                  {typeName(t)}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </section>
      </div>

      <Dialog open={moving !== null} onOpenChange={(o) => !o && setMoving(null)}>
        {moving && (
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Move {titleOf(moving)} to another host</DialogTitle>
            </DialogHeader>
            <div className="flex flex-col gap-3">
              <SelectField
                id="move-to"
                label="Host"
                value={moveTo}
                onChange={setMoveTo}
                options={[
                  ...others.map((h): [string, string] => [h.id, h.name]),
                  [NEW_HOST, 'New host'],
                ]}
              />
              {moveTo === NEW_HOST && (
                <>
                  <TextField id="move-name" label="Name" value={newName} onChange={setNewName} />
                  <TextField
                    id="move-address"
                    label="Address"
                    value={newAddress}
                    onChange={setNewAddress}
                  />
                </>
              )}
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => setMoving(null)}>
                Cancel
              </Button>
              <Button
                disabled={moveTo === NEW_HOST && (!newName.trim() || !newAddress.trim())}
                onClick={move}
              >
                Move
              </Button>
            </DialogFooter>
          </DialogContent>
        )}
      </Dialog>
    </PageShell>
  )
}
