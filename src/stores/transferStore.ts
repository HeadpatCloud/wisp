import { create } from 'zustand'

export type TransferDir = 'upload' | 'download'
export type TransferStatus = 'queued' | 'active' | 'done' | 'error'

export interface Transfer {
  id: string
  dir: TransferDir
  name: string
  transferred: number
  total: number
  status: TransferStatus
}

interface TransferState {
  transfers: Transfer[]
  start: (t: Transfer) => void
  activate: (id: string) => void
  progress: (id: string, transferred: number, total: number) => void
  finish: (id: string, error?: boolean) => void
  remove: (id: string) => void
  clearDone: () => void
}

export const useTransferStore = create<TransferState>()((set, get) => ({
  transfers: [],
  start: (t) => set({ transfers: [...get().transfers, t] }),
  activate: (id) =>
    set({
      transfers: get().transfers.map((t) => (t.id === id ? { ...t, status: 'active' } : t)),
    }),
  progress: (id, transferred, total) =>
    set({
      transfers: get().transfers.map((t) => (t.id === id ? { ...t, transferred, total } : t)),
    }),
  finish: (id, error) =>
    set({
      transfers: get().transfers.map((t) =>
        t.id === id ? { ...t, status: error ? 'error' : 'done' } : t,
      ),
    }),
  remove: (id) => set({ transfers: get().transfers.filter((t) => t.id !== id) }),
  clearDone: () => set({ transfers: get().transfers.filter((t) => t.status === 'active') }),
}))
