import { open, save } from '@tauri-apps/plugin-dialog'
import { commands, type Group, type ImportCandidate, type Profile } from '@/bindings'
import { unwrap } from '@/lib/ipc'

export const importSshConfig = async (path?: string): Promise<ImportCandidate[]> =>
  unwrap(await commands.importSshConfig(path ?? null))

// Returns false if the user cancelled the save dialog.
export async function exportProfilesToFile(): Promise<boolean> {
  const path = await save({
    defaultPath: 'wisp-profiles.json',
    filters: [{ name: 'JSON', extensions: ['json'] }],
  })
  if (!path) return false
  unwrap(await commands.exportProfiles(path))
  return true
}

// Returns the number of profiles imported, or null if the user cancelled.
export async function importProfilesFromFile(): Promise<number | null> {
  const path = await open({ filters: [{ name: 'JSON', extensions: ['json'] }] })
  if (typeof path !== 'string') return null
  return unwrap(await commands.importProfiles(path))
}

// Clone a profile for "Duplicate". New ids (profile + tunnels), a "(copy)" name, and
// no shared vault secret - the user re-enters the password on the copy.
export function duplicateProfile(p: Profile): Profile {
  return {
    ...p,
    id: crypto.randomUUID(),
    name: `${p.name} (copy)`,
    secretId: null,
    tunnels: (p.tunnels ?? []).map((t) => ({ ...t, id: crypto.randomUUID() })),
  }
}

// The next free order value at the end of a group (or the ungrouped list).
export function nextProfileOrder(profiles: Profile[], groupId: string | null): number {
  const orders = profiles.filter((p) => (p.groupId ?? null) === groupId).map((p) => p.order)
  return orders.length ? Math.max(...orders) + 1 : 0
}

export function nextGroupOrder(groups: Group[]): number {
  return groups.length ? Math.max(...groups.map((g) => g.order)) + 1 : 0
}

// Move/reorder a profile into `groupId` relative to `targetId` (placed before it, or after
// when placeAfter; appended if targetId is null). Returns only the profiles whose order or
// group changed, with fresh sequential orders, ready to persist.
export function reorderProfiles(
  profiles: Profile[],
  draggedId: string,
  groupId: string | null,
  targetId: string | null,
  placeAfter: boolean,
): Profile[] {
  const dragged = profiles.find((p) => p.id === draggedId)
  if (!dragged) return []
  const list = profiles
    .filter((p) => (p.groupId ?? null) === groupId && p.id !== draggedId)
    .sort((a, b) => a.order - b.order)
  let at = targetId ? list.findIndex((p) => p.id === targetId) : list.length
  if (at < 0) at = list.length
  if (placeAfter) at += 1
  list.splice(at, 0, dragged)
  const changed: Profile[] = []
  list.forEach((p, i) => {
    if (p.order !== i || (p.groupId ?? null) !== groupId) {
      changed.push({ ...p, order: i, groupId })
    }
  })
  return changed
}

// Reorder a group relative to `targetId`. Returns the groups whose order changed.
export function reorderGroups(
  groups: Group[],
  draggedId: string,
  targetId: string | null,
  placeAfter: boolean,
): Group[] {
  const dragged = groups.find((g) => g.id === draggedId)
  if (!dragged) return []
  const list = groups.filter((g) => g.id !== draggedId).sort((a, b) => a.order - b.order)
  let at = targetId ? list.findIndex((g) => g.id === targetId) : list.length
  if (at < 0) at = list.length
  if (placeAfter) at += 1
  list.splice(at, 0, dragged)
  const changed: Group[] = []
  list.forEach((g, i) => {
    if (g.order !== i) changed.push({ ...g, order: i })
  })
  return changed
}

// True if making `currentId` jump through `candidateId` would form a cycle
// (candidate's existing jump chain already leads back to current).
export function wouldCycle(
  profiles: Profile[],
  currentId: string | null,
  candidateId: string,
): boolean {
  let id: string | null = candidateId
  const seen = new Set<string>()
  while (id) {
    if (id === currentId || seen.has(id)) return true
    seen.add(id)
    id = profiles.find((p) => p.id === id)?.jumpHostId ?? null
  }
  return false
}
