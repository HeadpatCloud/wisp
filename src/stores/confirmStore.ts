import { create } from 'zustand'

export interface ConfirmInput {
  title: string
  body: string
  confirmLabel: string
  destructive?: boolean
}

interface ConfirmState {
  request: (ConfirmInput & { resolve: (ok: boolean) => void }) | null
  ask: (c: ConfirmInput) => Promise<boolean>
  answer: (ok: boolean) => void
}

export const useConfirmStore = create<ConfirmState>()((set, get) => ({
  request: null,
  ask: (c) =>
    new Promise<boolean>((resolve) => {
      // A newer question replaces one still open, which counts as Cancel.
      get().request?.resolve(false)
      set({ request: { ...c, resolve } })
    }),
  answer: (ok) => {
    const r = get().request
    set({ request: null })
    r?.resolve(ok)
  },
}))

export const confirm = (c: ConfirmInput) => useConfirmStore.getState().ask(c)
