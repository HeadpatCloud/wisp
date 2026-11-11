import { useState } from 'react'
import type { AppError, Tunnel } from '@/bindings'
import { HostKeyDialog, type HostKeyPrompt } from '@/features/sessions/HostKeyDialog'
import { connectErrorText } from '@/lib/ipc'
import { trustHostKey } from '@/lib/ssh'
import { useTunnelStore } from '@/stores/tunnelStore'
import { runTunnel, tunnelNotice } from './run'

function hostKeyPrompt(e: unknown): HostKeyPrompt | null {
  const err = e as AppError
  if (err && typeof err === 'object' && 'kind' in err) {
    if (err.kind === 'hostKeyUnknown') return { kind: 'unknown', ...err.message }
    if (err.kind === 'hostKeyMismatch') {
      const { host, port, stored, offered } = err.message
      return { kind: 'mismatch', host, port, stored, offered }
    }
  }
  return null
}

// A start that never reached a session still shows its reason where the tunnel is listed.
function failed(tunnel: Tunnel, e: unknown) {
  useTunnelStore.getState().start({
    tunnelId: tunnel.id,
    sessionId: '',
    state: 'error',
    bytesUp: 0,
    bytesDown: 0,
    message: connectErrorText(e),
  })
  tunnelNotice(tunnel, `Couldn't start: ${connectErrorText(e)}`)
}

// Starts a tunnel and, when its background connection meets an unknown host key, asks first.
// Render `dialog` wherever `start` is used.
export function useTunnelStarter() {
  const [prompt, setPrompt] = useState<{
    p: HostKeyPrompt
    tunnel: Tunnel
    retry: () => void
  } | null>(null)
  const start = (connectionId: string, tunnel: Tunnel) => {
    const go = () =>
      runTunnel(connectionId, tunnel).catch((e: unknown) => {
        const p = hostKeyPrompt(e)
        if (p) setPrompt({ p, tunnel, retry: go })
        else failed(tunnel, e)
      })
    void go()
  }
  const dialog = (
    <HostKeyDialog
      prompt={prompt?.p ?? null}
      onAccept={async () => {
        const p = prompt
        setPrompt(null)
        if (!p) return
        const fingerprint = p.p.kind === 'unknown' ? p.p.fingerprint : p.p.offered
        await trustHostKey(p.p.host, p.p.port, fingerprint).then(p.retry, (e: unknown) =>
          failed(p.tunnel, e),
        )
      }}
      onReject={() => setPrompt(null)}
    />
  )
  return { start, dialog }
}
