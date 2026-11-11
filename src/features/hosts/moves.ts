import type { Folder, FolderMove, Host, HostMove } from '@/bindings'
import type { DropTarget } from './usePointerDrag'

const endOf = (orders: number[]) => (orders.length ? Math.max(...orders) + 1 : 0)

// Where a dragged host lands: at the end of a folder, or next to another host, with the hosts of
// that folder numbered again so the order stays plain.
export function hostDropMoves(hosts: Host[], hostId: string, target: DropTarget): HostMove[] {
  if (target.kind === 'conn') return []
  if (target.kind === 'folder') {
    const order = endOf(
      hosts.filter((h) => h.folderId === target.id && h.id !== hostId).map((h) => h.order),
    )
    return [{ id: hostId, folderId: target.id, order }]
  }
  if (target.hostId === hostId) return []
  const anchor = hosts.find((h) => h.id === target.hostId)
  if (!anchor) return []
  const folderId = anchor.folderId
  const row = hosts
    .filter((h) => h.folderId === folderId && h.id !== hostId)
    .sort((a, b) => a.order - b.order)
    .map((h) => h.id)
  const at = row.indexOf(anchor.id) + (target.kind === 'after' ? 1 : 0)
  row.splice(at, 0, hostId)
  return row.map((id, order) => ({ id, folderId, order }))
}

export function folderDropMove(
  folders: Folder[],
  folderId: string,
  target: DropTarget,
): FolderMove | null {
  if (target.kind !== 'folder' || target.id === folderId) return null
  const order = endOf(folders.filter((f) => f.parentId === target.id).map((f) => f.order))
  return { id: folderId, parentId: target.id, order }
}

// The folder before or after `current` in nav order, where null (no folder) comes last, as on
// the page. undefined means there is none in that direction.
export function neighbourFolder(
  navOrder: string[],
  current: string | null,
  step: 1 | -1,
): string | null | undefined {
  const all: (string | null)[] = [...navOrder, null]
  const at = all.indexOf(current)
  const next = at + step
  return next >= 0 && next < all.length ? all[next] : undefined
}
