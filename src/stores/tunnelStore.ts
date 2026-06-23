import { create } from 'zustand'

export type TunnelState = 'starting' | 'active' | 'error'

export interface TunnelRuntime {
  tunnelId: string
  sessionId: string
  state: TunnelState
  bytesUp: number
  bytesDown: number
  message?: string
}

interface TunnelStoreState {
  byId: Record<string, TunnelRuntime>
  start: (rt: TunnelRuntime) => void
  setStatus: (s: {
    tunnelId: string
    state: TunnelState
    bytesUp: number
    bytesDown: number
    message?: string | null
  }) => void
  remove: (tunnelId: string) => void
  clearSession: (sessionId: string) => void
}

export const useTunnelStore = create<TunnelStoreState>()((set, get) => ({
  byId: {},
  start: (rt) => set({ byId: { ...get().byId, [rt.tunnelId]: rt } }),
  setStatus: ({ tunnelId, state, bytesUp, bytesDown, message }) => {
    const prev = get().byId[tunnelId]
    if (!prev) return
    set({
      byId: {
        ...get().byId,
        [tunnelId]: { ...prev, state, bytesUp, bytesDown, message: message ?? undefined },
      },
    })
  },
  remove: (tunnelId) => {
    const next = { ...get().byId }
    delete next[tunnelId]
    set({ byId: next })
  },
  clearSession: (sessionId) =>
    set({
      byId: Object.fromEntries(
        Object.entries(get().byId).filter(([, t]) => t.sessionId !== sessionId),
      ),
    }),
}))
