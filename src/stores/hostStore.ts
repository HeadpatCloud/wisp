import { create } from 'zustand'
import {
  type ConnectionMove,
  commands,
  type Folder,
  type FolderMove,
  type Host,
  type HostMove,
  type SavedLogin,
  type ShellInfo,
  type SshKey,
  type Tag,
} from '@/bindings'
import { unwrap } from '@/lib/ipc'
import { listShells } from '@/lib/local'

interface HostState {
  folders: Folder[]
  hosts: Host[]
  logins: SavedLogin[]
  keys: SshKey[]
  tags: Tag[]
  shells: ShellInfo[]
  loaded: boolean
  load: () => Promise<void>
  loadShells: () => Promise<void>
  // Returns the pending-delete ids of connections the host no longer has.
  saveHost: (host: Host, moves?: ConnectionMove[]) => Promise<string[]>
  saveFolder: (folder: Folder) => Promise<void>
  // Returns the pending-delete id for the Undo notice.
  deleteHost: (id: string) => Promise<string>
  undoDelete: (pendingId: string) => Promise<void>
  purgeDeletes: (pendingIds: string[]) => Promise<void>
  moveHosts: (moves: HostMove[]) => Promise<void>
  moveFolders: (moves: FolderMove[]) => Promise<void>
  saveLogin: (login: SavedLogin) => Promise<void>
  // Refused while a connection uses it; the error names them.
  deleteLogin: (id: string) => Promise<void>
  saveKey: (key: SshKey) => Promise<void>
  deleteKey: (id: string) => Promise<void>
  saveTag: (tag: Tag) => Promise<void>
  // Also takes the tag off every host.
  deleteTag: (id: string) => Promise<void>
}

export const useHostStore = create<HostState>()((set, get) => ({
  folders: [],
  hosts: [],
  logins: [],
  keys: [],
  tags: [],
  shells: [],
  loaded: false,
  load: async () => {
    const file = unwrap(await commands.getHosts())
    set({
      folders: file.folders ?? [],
      hosts: file.hosts ?? [],
      logins: file.logins ?? [],
      keys: file.keys ?? [],
      tags: file.tags ?? [],
      loaded: true,
    })
  },
  loadShells: async () => {
    set({ shells: await listShells() })
  },
  saveHost: async (host, moves = []) => {
    const pending = unwrap(await commands.upsertHost(host, moves))
    await get().load()
    return pending
  },
  deleteHost: async (id) => {
    const pending = unwrap(await commands.deleteHost(id))
    await get().load()
    return pending
  },
  undoDelete: async (pendingId) => {
    unwrap(await commands.undoDelete(pendingId))
    await get().load()
  },
  purgeDeletes: async (pendingIds) => {
    unwrap(await commands.purgeDeletes(pendingIds))
  },
  saveFolder: async (folder) => {
    unwrap(await commands.upsertFolder(folder))
    await get().load()
  },
  saveLogin: async (login) => {
    unwrap(await commands.upsertLogin(login))
    await get().load()
  },
  deleteLogin: async (id) => {
    unwrap(await commands.deleteLogin(id))
    await get().load()
  },
  saveKey: async (key) => {
    unwrap(await commands.upsertKey(key))
    await get().load()
  },
  deleteKey: async (id) => {
    unwrap(await commands.deleteKey(id))
    await get().load()
  },
  saveTag: async (tag) => {
    unwrap(await commands.upsertTag(tag))
    await get().load()
  },
  deleteTag: async (id) => {
    unwrap(await commands.deleteTag(id))
    await get().load()
  },
  moveHosts: async (moves) => {
    unwrap(await commands.moveHosts(moves))
    await get().load()
  },
  moveFolders: async (moves) => {
    unwrap(await commands.moveFolders(moves))
    await get().load()
  },
}))
