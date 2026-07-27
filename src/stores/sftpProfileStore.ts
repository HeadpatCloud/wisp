import { create } from 'zustand'
import { commands, type SftpProfile } from '@/bindings'
import { unwrap } from '@/lib/ipc'

interface SftpProfileState {
  profiles: SftpProfile[]
  loaded: boolean
  load: () => Promise<void>
  save: (profile: SftpProfile) => Promise<void>
  remove: (id: string) => Promise<void>
}

// Every secret the profile references: the password/passphrase plus one per key.
function secretIds(p: SftpProfile): string[] {
  return [p.secretId, ...(p.keys ?? []).map((k) => k.secretId)].filter((id): id is string => !!id)
}

export const useSftpProfileStore = create<SftpProfileState>()((set, get) => ({
  profiles: [],
  loaded: false,
  load: async () => {
    set({ profiles: unwrap(await commands.listSftpProfiles()), loaded: true })
  },
  save: async (profile) => {
    unwrap(await commands.upsertSftpProfile(profile))
    set({ profiles: unwrap(await commands.listSftpProfiles()) })
  },
  remove: async (id) => {
    const target = get().profiles.find((p) => p.id === id)
    // Best effort: a secret that's already gone must not block deleting the profile.
    for (const secretId of target ? secretIds(target) : []) {
      await commands.deleteSecret(secretId).catch(() => {})
    }
    unwrap(await commands.deleteSftpProfile(id))
    set({ profiles: unwrap(await commands.listSftpProfiles()) })
  },
}))
