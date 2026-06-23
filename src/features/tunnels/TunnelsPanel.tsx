import type { Tunnel } from '@/bindings'
import { Button } from '@/components/ui/button'
import { formatBytes } from '@/lib/format'
import { startTunnel, stopTunnel } from '@/lib/tunnels'
import { cn } from '@/lib/utils'
import { useTunnelStore } from '@/stores/tunnelStore'

export function TunnelsPanel({
  sessionId,
  profileTunnels,
}: {
  sessionId: string
  profileTunnels: Tunnel[]
}) {
  const byId = useTunnelStore((s) => s.byId)
  const startRt = useTunnelStore((s) => s.start)
  const removeRt = useTunnelStore((s) => s.remove)

  if (profileTunnels.length === 0) {
    return (
      <div className="p-2 text-muted-foreground text-xs">
        No tunnels configured for this profile.
      </div>
    )
  }

  return (
    <div className="space-y-1 p-2">
      {profileTunnels.map((t) => {
        const rt = byId[t.id]
        const on = rt?.state === 'active' || rt?.state === 'starting'
        return (
          <div key={t.id} className="flex items-center gap-2 text-sm">
            <span
              className={cn(
                'size-1.5 shrink-0 rounded-full bg-muted',
                rt?.state === 'active' && 'bg-green-500',
                rt?.state === 'starting' && 'bg-yellow-500',
                rt?.state === 'error' && 'bg-red-500',
              )}
            />
            <span className="min-w-0 flex-1 truncate">
              {t.kind} {t.bindHost}:{t.bindPort}
              {t.targetHost ? ` -> ${t.targetHost}:${t.targetPort}` : ''}
            </span>
            {rt?.state === 'active' && (
              <span className="shrink-0 text-muted-foreground text-xs tabular-nums">
                ↑{formatBytes(rt.bytesUp)} ↓{formatBytes(rt.bytesDown)}
              </span>
            )}
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={async () => {
                if (on) {
                  await stopTunnel(t.id)
                  removeRt(t.id)
                } else {
                  startRt({
                    tunnelId: t.id,
                    sessionId,
                    state: 'starting',
                    bytesUp: 0,
                    bytesDown: 0,
                  })
                  try {
                    await startTunnel(sessionId, t)
                  } catch (e) {
                    useTunnelStore.getState().setStatus({
                      tunnelId: t.id,
                      state: 'error',
                      bytesUp: 0,
                      bytesDown: 0,
                      message: String(e),
                    })
                  }
                }
              }}
            >
              {on ? 'Stop' : 'Start'}
            </Button>
          </div>
        )
      })}
    </div>
  )
}
