import { create } from 'zustand'
import { commands, type VncProfile } from '@/bindings'
import { unwrap } from '@/lib/ipc'

interface VncProfileState {
  profiles: VncProfile[]
  loaded: boolean
  load: () => Promise<void>
  save: (profile: VncProfile) => Promise<void>
  remove: (id: string) => Promise<void>
}

export const useVncProfileStore = create<VncProfileState>()((set, get) => ({
  profiles: [],
  loaded: false,
  load: async () => {
    set({ profiles: unwrap(await commands.listVncProfiles()), loaded: true })
  },
  save: async (profile) => {
    unwrap(await commands.upsertVncProfile(profile))
    set({ profiles: unwrap(await commands.listVncProfiles()) })
  },
  remove: async (id) => {
    const target = get().profiles.find((p) => p.id === id)
    // Best effort: a secret that's already gone must not block deleting the profile.
    if (target?.secretId) await commands.deleteSecret(target.secretId).catch(() => {})
    unwrap(await commands.deleteVncProfile(id))
    set({ profiles: unwrap(await commands.listVncProfiles()) })
  },
}))
