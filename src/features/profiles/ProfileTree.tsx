import {
  ChevronDown,
  ChevronRight,
  Cloud,
  FolderPlus,
  FolderTree,
  Monitor,
  Network,
  Plus,
  Server,
  SquareTerminal,
} from 'lucide-react'
import type { ReactNode, PointerEvent as ReactPointerEvent } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { Group, Profile, S3Profile, ShellInfo } from '@/bindings'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { duplicateProfile, reorderGroups, reorderProfiles } from '@/lib/profiles'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { ProfileIcon } from './ProfileIcon'

const COLLAPSED_KEY = 'sidebar-collapsed'

function loadCollapsed(): Record<string, boolean> {
  try {
    return JSON.parse(localStorage.getItem(COLLAPSED_KEY) || '{}')
  } catch {
    return {}
  }
}

type DragKind = 'profile' | 'group'
type Hint = { id: string; after: boolean } | null

// Resolve what's under the pointer into a drop intent. Pointer events are used instead of
// the HTML5 drag API because Tauri's webview drag-drop (needed for file uploads) suppresses
// HTML5 drag events on Windows.
type Drop =
  | { type: 'profile'; id: string; groupId: string | null; after: boolean }
  | { type: 'group'; id: string; after: boolean }
  | { type: 'group-area'; groupId: string }
  | { type: 'tree' }
  | null

function resolveDrop(x: number, y: number): Drop {
  const el = document.elementFromPoint(x, y) as HTMLElement | null
  if (!el) return null
  const row = el.closest('[data-row-id]') as HTMLElement | null
  if (row) {
    const rect = row.getBoundingClientRect()
    const after = y > rect.top + rect.height / 2
    const id = row.dataset.rowId ?? ''
    if (row.dataset.rowKind === 'group') return { type: 'group', id, after }
    return { type: 'profile', id, groupId: row.dataset.groupId || null, after }
  }
  const area = el.closest('[data-group-area]') as HTMLElement | null
  if (area) return { type: 'group-area', groupId: area.dataset.groupArea ?? '' }
  if (el.closest('[data-tree]')) return { type: 'tree' }
  return null
}

interface ProfileTreeProps {
  onActivateProfile: (profile: Profile) => void
  onNewProfile: () => void
  onNewVnc: () => void
  onNewFtp: () => void
  onNewS3: () => void
  onActivateS3: (profile: S3Profile) => void
  onEditS3: (profile: S3Profile) => void
  onNewLocalShell: (program: string | null, title: string) => void
  onNewSftp: (profileId: string, title: string) => void
  onOpenSftpPicker: () => void
  onEditProfile: (profile: Profile) => void
  onNewGroup: () => void
  onEditGroup: (group: Group) => void
  shells: ShellInfo[]
}

export function ProfileTree({
  onActivateProfile,
  onNewProfile,
  onNewVnc,
  onNewFtp,
  onNewS3,
  onActivateS3,
  onEditS3,
  onNewLocalShell,
  onNewSftp,
  onOpenSftpPicker,
  onEditProfile,
  onNewGroup,
  onEditGroup,
  shells,
}: ProfileTreeProps) {
  const groups = useProfileStore((s) => s.groups)
  const profiles = useProfileStore((s) => s.profiles)
  const s3Profiles = useS3ProfileStore((s) => s.profiles)
  const removeS3 = useS3ProfileStore((s) => s.remove)
  const removeProfile = useProfileStore((s) => s.removeProfile)
  const removeGroup = useProfileStore((s) => s.removeGroup)
  const saveProfile = useProfileStore((s) => s.saveProfile)
  const saveProfiles = useProfileStore((s) => s.saveProfiles)
  const saveGroups = useProfileStore((s) => s.saveGroups)
  const [query, setQuery] = useState('')
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(loadCollapsed)
  const [dragId, setDragId] = useState<string | null>(null)
  const [hint, setHint] = useState<Hint>(null)

  const dragRef = useRef<{
    kind: DragKind
    id: string
    sx: number
    sy: number
    active: boolean
  } | null>(null)
  const hintRef = useRef<Hint>(null)
  const suppressClick = useRef(false)
  const profilesRef = useRef(profiles)
  profilesRef.current = profiles
  const groupsRef = useRef(groups)
  groupsRef.current = groups

  const toggleGroup = (id: string) => {
    if (suppressClick.current) {
      suppressClick.current = false
      return
    }
    setCollapsed((c) => {
      const next = { ...c, [id]: !c[id] }
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify(next))
      return next
    })
  }

  const beginDrag = (kind: DragKind, id: string) => (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    dragRef.current = { kind, id, sx: e.clientX, sy: e.clientY, active: false }
  }

  useEffect(() => {
    const setHintSafe = (h: Hint) => {
      if (hintRef.current?.id === h?.id && hintRef.current?.after === h?.after) return
      hintRef.current = h
      setHint(h)
    }
    const onMove = (e: PointerEvent) => {
      const d = dragRef.current
      if (!d) return
      if (!d.active) {
        if (Math.abs(e.clientX - d.sx) < 5 && Math.abs(e.clientY - d.sy) < 5) return
        d.active = true
        setDragId(d.id)
      }
      const drop = resolveDrop(e.clientX, e.clientY)
      if (drop?.type === d.kind && drop.id !== d.id) setHintSafe({ id: drop.id, after: drop.after })
      else setHintSafe(null)
    }
    const onUp = (e: PointerEvent) => {
      const d = dragRef.current
      dragRef.current = null
      if (d?.active) {
        suppressClick.current = true
        const drop = resolveDrop(e.clientX, e.clientY)
        if (d.kind === 'profile') {
          let changed: Profile[] = []
          if (drop?.type === 'profile' && drop.id !== d.id) {
            changed = reorderProfiles(profilesRef.current, d.id, drop.groupId, drop.id, drop.after)
          } else if (drop?.type === 'group') {
            changed = reorderProfiles(profilesRef.current, d.id, drop.id, null, false)
          } else if (drop?.type === 'group-area') {
            changed = reorderProfiles(profilesRef.current, d.id, drop.groupId, null, false)
          } else if (drop?.type === 'tree') {
            changed = reorderProfiles(profilesRef.current, d.id, null, null, false)
          }
          if (changed.length) saveProfiles(changed)
        } else if (drop?.type === 'group' && drop.id !== d.id) {
          const changed = reorderGroups(groupsRef.current, d.id, drop.id, drop.after)
          if (changed.length) saveGroups(changed)
        }
      }
      setDragId(null)
      setHintSafe(null)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    return () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
    }
  }, [saveProfiles, saveGroups])

  const dropLine = (id: string): 'top' | 'bottom' | null =>
    hint?.id === id ? (hint.after ? 'bottom' : 'top') : null

  const sortedGroups = useMemo(() => [...groups].sort((a, b) => a.order - b.order), [groups])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    const list = q
      ? profiles.filter((p) => p.name.toLowerCase().includes(q) || p.host.toLowerCase().includes(q))
      : profiles
    return [...list].sort((a, b) => a.order - b.order)
  }, [profiles, query])

  const ungrouped = filtered.filter((p) => !p.groupId)

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 p-2">
        <Input
          placeholder="Search..."
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="h-8"
        />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label="New connection"
              className="rounded p-1.5 hover:bg-muted"
            >
              <Plus className="size-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onNewProfile}>
              <Server /> SSH profile
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onOpenSftpPicker}>
              <FolderTree /> SFTP connection
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onNewFtp}>
              <Network /> FTP / FTPS
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onNewVnc}>
              <Monitor /> VNC connection
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onNewS3}>
              <Cloud /> S3 connection
            </DropdownMenuItem>
            {shells.length > 1 ? (
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <SquareTerminal /> Local shell
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  {shells.map((s) => (
                    <DropdownMenuItem
                      key={s.program}
                      onSelect={() => onNewLocalShell(s.program, s.name)}
                    >
                      {s.name}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            ) : (
              <DropdownMenuItem
                onSelect={() => onNewLocalShell(shells[0]?.program ?? null, 'Local shell')}
              >
                <SquareTerminal /> Local shell
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        <button
          type="button"
          aria-label="New group"
          onClick={onNewGroup}
          className="rounded p-1.5 hover:bg-muted"
        >
          <FolderPlus className="size-4" />
        </button>
      </div>

      <div data-tree className="min-h-0 flex-1 overflow-y-auto px-1 pb-2">
        {groups.length === 0 && profiles.length === 0 && s3Profiles.length === 0 && (
          <div className="px-2 py-6 text-center text-muted-foreground text-xs">
            No hosts yet. Use + to add a profile, or Import from ~/.ssh/config.
          </div>
        )}
        {sortedGroups.map((group) => {
          const members = filtered.filter((p) => p.groupId === group.id)
          const isCollapsed = collapsed[group.id]
          return (
            <div key={group.id} data-group-area={group.id}>
              <Row onEdit={() => onEditGroup(group)} onDelete={() => removeGroup(group.id)}>
                <button
                  type="button"
                  data-row-id={group.id}
                  data-row-kind="group"
                  onPointerDown={beginDrag('group', group.id)}
                  onClick={() => toggleGroup(group.id)}
                  className={`flex w-full select-none items-center gap-1.5 rounded border-2 border-transparent px-1.5 py-1 text-sm hover:bg-muted ${dragId === group.id ? 'opacity-40' : ''} ${dropLine(group.id) === 'top' ? 'border-t-primary' : dropLine(group.id) === 'bottom' ? 'border-b-primary' : ''}`}
                >
                  {isCollapsed ? (
                    <ChevronRight className="size-3.5" />
                  ) : (
                    <ChevronDown className="size-3.5" />
                  )}
                  <ProfileIcon icon={group.icon} className="size-4 text-muted-foreground" />
                  <span className="truncate font-medium">{group.name}</span>
                </button>
              </Row>
              {!isCollapsed &&
                members.map((p) => (
                  <ProfileRow
                    key={p.id}
                    profile={p}
                    onActivate={() => onActivateProfile(p)}
                    onEdit={() => onEditProfile(p)}
                    onDelete={() => removeProfile(p.id)}
                    onOpenSftp={() => onNewSftp(p.id, p.name)}
                    onBeginDrag={beginDrag('profile', p.id)}
                    dragging={dragId === p.id}
                    dropLine={dropLine(p.id)}
                    onDuplicate={async () => {
                      const copy = duplicateProfile(p)
                      await saveProfile(copy)
                      onEditProfile(copy)
                    }}
                    indented
                  />
                ))}
            </div>
          )
        })}
        {ungrouped.map((p) => (
          <ProfileRow
            key={p.id}
            profile={p}
            onActivate={() => onActivateProfile(p)}
            onEdit={() => onEditProfile(p)}
            onDelete={() => removeProfile(p.id)}
            onOpenSftp={() => onNewSftp(p.id, p.name)}
            onBeginDrag={beginDrag('profile', p.id)}
            dragging={dragId === p.id}
            dropLine={dropLine(p.id)}
            onDuplicate={async () => {
              const copy = duplicateProfile(p)
              await saveProfile(copy)
              onEditProfile(copy)
            }}
          />
        ))}
        {s3Profiles.length > 0 && (
          <div className="mt-2">
            <div className="px-2 py-1 font-medium text-muted-foreground text-xs">S3</div>
            {s3Profiles
              .filter((p) => {
                const q = query.trim().toLowerCase()
                return (
                  !q || p.name.toLowerCase().includes(q) || p.endpoint.toLowerCase().includes(q)
                )
              })
              .map((p) => (
                <Row key={p.id} onEdit={() => onEditS3(p)} onDelete={() => removeS3(p.id)}>
                  <button
                    type="button"
                    onDoubleClick={() => onActivateS3(p)}
                    className="flex w-full select-none items-center gap-1.5 rounded border-2 border-transparent px-1.5 py-1 text-sm hover:bg-muted"
                  >
                    <Cloud className="size-4 text-muted-foreground" />
                    <span className="truncate">{p.name}</span>
                    <span className="ml-auto truncate text-muted-foreground text-xs">
                      {p.endpoint}
                    </span>
                  </button>
                </Row>
              ))}
          </div>
        )}
      </div>
    </div>
  )
}

function ProfileRow({
  profile,
  onActivate,
  onEdit,
  onDelete,
  onDuplicate,
  onOpenSftp,
  onBeginDrag,
  dragging,
  dropLine,
  indented,
}: {
  profile: Profile
  onActivate: () => void
  onEdit: () => void
  onDelete: () => void
  onDuplicate: () => void
  onOpenSftp: () => void
  onBeginDrag: (e: ReactPointerEvent) => void
  dragging?: boolean
  dropLine?: 'top' | 'bottom' | null
  indented?: boolean
}) {
  return (
    <Row onEdit={onEdit} onDelete={onDelete} onDuplicate={onDuplicate} onOpenSftp={onOpenSftp}>
      <button
        type="button"
        data-row-id={profile.id}
        data-row-kind="profile"
        data-group-id={profile.groupId ?? ''}
        onPointerDown={onBeginDrag}
        onDoubleClick={onActivate}
        className={`flex w-full select-none items-center gap-1.5 rounded border-2 border-transparent px-1.5 py-1 text-sm hover:bg-muted ${indented ? 'pl-6' : ''} ${dragging ? 'opacity-40' : ''} ${dropLine === 'top' ? 'border-t-primary' : dropLine === 'bottom' ? 'border-b-primary' : ''}`}
      >
        <ProfileIcon icon={profile.icon} className="size-4 text-muted-foreground" />
        <span className="truncate">{profile.name}</span>
        <span className="ml-auto truncate text-muted-foreground text-xs">{profile.host}</span>
      </button>
    </Row>
  )
}

function Row({
  children,
  onEdit,
  onDelete,
  onDuplicate,
  onOpenSftp,
}: {
  children: ReactNode
  onEdit: () => void
  onDelete: () => void
  onDuplicate?: () => void
  onOpenSftp?: () => void
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={onEdit}>Edit</ContextMenuItem>
        {onOpenSftp && <ContextMenuItem onSelect={onOpenSftp}>Open SFTP</ContextMenuItem>}
        {onDuplicate && <ContextMenuItem onSelect={onDuplicate}>Duplicate</ContextMenuItem>}
        <ContextMenuItem onSelect={onDelete} className="text-red-600">
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
