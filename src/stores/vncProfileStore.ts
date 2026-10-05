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
    // No reload: one that failed would report a profile that was saved as not saved.
    const { profiles } = get()
    set({
      profiles: profiles.some((p) => p.id === profile.id)
        ? profiles.map((p) => (p.id === profile.id ? profile : p))
        : [...profiles, profile],
    })
  },
  remove: async (id) => {
    const target = get().profiles.find((p) => p.id === id)
    unwrap(await commands.deleteVncProfile(id))
    // Only once the profile is gone: one that could not be deleted still needs its password.
    // Best effort, a secret left behind in the vault does no harm.
    if (target?.secretId) await commands.deleteSecret(target.secretId).catch(() => {})
    set({ profiles: unwrap(await commands.listVncProfiles()) })
  },
}))
