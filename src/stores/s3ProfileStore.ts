import { create } from 'zustand'
import { commands, type S3Profile } from '@/bindings'
import { unwrap } from '@/lib/ipc'

interface S3ProfileState {
  profiles: S3Profile[]
  loaded: boolean
  load: () => Promise<void>
  save: (profile: S3Profile) => Promise<void>
  remove: (id: string) => Promise<void>
}

export const useS3ProfileStore = create<S3ProfileState>()((set, get) => ({
  profiles: [],
  loaded: false,
  load: async () => {
    set({ profiles: unwrap(await commands.listS3Profiles()), loaded: true })
  },
  save: async (profile) => {
    unwrap(await commands.upsertS3Profile(profile))
    set({ profiles: unwrap(await commands.listS3Profiles()) })
  },
  remove: async (id) => {
    const target = get().profiles.find((p) => p.id === id)
    if (target?.secretId) unwrap(await commands.deleteSecret(target.secretId))
    unwrap(await commands.deleteS3Profile(id))
    set({ profiles: unwrap(await commands.listS3Profiles()) })
  },
}))
