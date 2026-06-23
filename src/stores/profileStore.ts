import { create } from 'zustand'
import { commands, type Group, type Profile } from '@/bindings'
import { unwrap } from '@/lib/ipc'

interface ProfileState {
  groups: Group[]
  profiles: Profile[]
  loaded: boolean
  load: () => Promise<void>
  saveGroup: (group: Group) => Promise<void>
  saveGroups: (groups: Group[]) => Promise<void>
  removeGroup: (id: string) => Promise<void>
  saveProfile: (profile: Profile) => Promise<void>
  saveProfiles: (profiles: Profile[]) => Promise<void>
  removeProfile: (id: string) => Promise<void>
}

export const useProfileStore = create<ProfileState>()((set, get) => ({
  groups: [],
  profiles: [],
  loaded: false,
  load: async () => {
    const groups = unwrap(await commands.listGroups())
    const profiles = unwrap(await commands.listProfiles())
    set({ groups, profiles, loaded: true })
  },
  saveGroup: async (group) => {
    unwrap(await commands.upsertGroup(group))
    set({ groups: unwrap(await commands.listGroups()) })
  },
  saveGroups: async (groups) => {
    for (const g of groups) unwrap(await commands.upsertGroup(g))
    set({ groups: unwrap(await commands.listGroups()) })
  },
  removeGroup: async (id) => {
    unwrap(await commands.deleteGroup(id))
    set({
      groups: unwrap(await commands.listGroups()),
      profiles: unwrap(await commands.listProfiles()),
    })
  },
  saveProfile: async (profile) => {
    unwrap(await commands.upsertProfile(profile))
    set({ profiles: unwrap(await commands.listProfiles()) })
  },
  saveProfiles: async (profiles) => {
    for (const p of profiles) unwrap(await commands.upsertProfile(p))
    set({ profiles: unwrap(await commands.listProfiles()) })
  },
  removeProfile: async (id) => {
    const target = get().profiles.find((p) => p.id === id)
    if (target?.secretId) {
      unwrap(await commands.deleteSecret(target.secretId))
    }
    unwrap(await commands.deleteProfile(id))
    set({ profiles: unwrap(await commands.listProfiles()) })
  },
}))
