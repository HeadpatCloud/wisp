import { create } from 'zustand'
import type { ParsedAddress } from '@/lib/address'
import { type NewConnectionPrefill, useHostsUiStore } from '@/stores/hostsUiStore'

// The address App's quick-connect dialogs open with; null when none was asked for.
export const useConnectOnce = create<{ request: ParsedAddress | null; clear: () => void }>()(
  (set) => ({
    request: null,
    clear: () => set({ request: null }),
  }),
)

export function prefillOf(p: ParsedAddress): NewConnectionPrefill {
  return {
    type: p.type === 'ftps' ? 'ftp' : p.type,
    address: p.host,
    ...(p.port !== null && { port: p.port }),
    ...(p.user !== null && { user: p.user }),
    ...(p.bucket !== null && { bucket: p.bucket }),
  }
}

// There is no quick connect for S3, so it goes to New connection, filled in.
export function connectOnce(p: ParsedAddress) {
  if (p.type === 's3') {
    useHostsUiStore.getState().openNewConnection(prefillOf(p))
    return
  }
  useConnectOnce.setState({ request: p })
}
