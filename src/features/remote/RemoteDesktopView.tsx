import {
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import type { AppError } from '@/bindings'
import { HostKeyDialog, type HostKeyPrompt } from '@/features/sessions/HostKeyDialog'
import type { RemoteDriver } from '@/lib/remoteDriver'
import type { FrameMessage } from '@/lib/remoteFrames'
import { detectPlatform, pointerButtons, RemoteKeyboard, WheelSteps } from '@/lib/remoteInput'
import { trustHostKey } from '@/lib/ssh'
import { cn } from '@/lib/utils'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'

type State =
  | { status: 'connecting' }
  | { status: 'connected' }
  | { status: 'closed'; reason: string }
  | { status: 'failed'; error: string }
  | { status: 'trust'; prompt: HostKeyPrompt }

interface Live {
  id: string
  keyboard: RemoteKeyboard
  wheel: WheelSteps
  buttons: number
  x: number
  y: number
  move: number
  synced: string
  pending: string | null
  received: number
}

const PICTURE_ERROR = 'The picture could not be updated.'

function pointOn(canvas: HTMLCanvasElement, e: { clientX: number; clientY: number }) {
  const rect = canvas.getBoundingClientRect()
  if (canvas.width === 0 || canvas.height === 0 || rect.width === 0 || rect.height === 0) {
    return null
  }
  // object-contain letterboxes the picture when the box does not have its shape.
  const scale = Math.min(rect.width / canvas.width, rect.height / canvas.height)
  const left = rect.left + (rect.width - canvas.width * scale) / 2
  const top = rect.top + (rect.height - canvas.height * scale) / 2
  const x = Math.floor((e.clientX - left) / scale)
  const y = Math.floor((e.clientY - top) / scale)
  return {
    x: Math.min(Math.max(x, 0), canvas.width - 1),
    y: Math.min(Math.max(y, 0), canvas.height - 1),
  }
}

function sendPointer(driver: RemoteDriver, live: Live, buttons: number) {
  cancelAnimationFrame(live.move)
  live.move = 0
  driver.pointer(live.id, buttons, live.x, live.y)
}

function releaseButtons(driver: RemoteDriver, live: Live) {
  if (live.buttons === 0) return
  live.buttons = 0
  sendPointer(driver, live, 0)
}

function writeClipboard(live: Live, text: string) {
  live.pending = text
  navigator.clipboard.writeText(text).then(
    () => {
      live.synced = text
      if (live.pending === text) live.pending = null
    },
    // Refused while the window is not focused; it stays pending for the next focus.
    () => {},
  )
}

export function RemoteDesktopView({
  tabId,
  driver,
  active,
}: {
  tabId: string
  driver: RemoteDriver
  active: boolean
}) {
  const [state, setState] = useState<State>({ status: 'connecting' })
  const [fullscreen, setFullscreen] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const reconnectRef = useRef<HTMLButtonElement>(null)
  const liveRef = useRef<Live | null>(null)
  const stopRef = useRef(() => {})
  const attemptRef = useRef(0)
  const removeTab = useSessionStore((s) => s.removeTab)

  const connect = useCallback(() => {
    stopRef.current()
    attemptRef.current += 1
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) {
      setState({ status: 'failed', error: PICTURE_ERROR })
      return
    }
    setState({ status: 'connecting' })
    let ended = false
    let sessionId: string | null = null
    let live: Live | null = null
    const queue: FrameMessage[] = []

    const release = () => {
      if (!live) return
      cancelAnimationFrame(live.move)
      live.keyboard.releaseAll()
      live = null
      liveRef.current = null
      canvas.style.cursor = ''
      canvas.blur()
    }

    const stop = () => {
      ended = true
      release()
      if (sessionId) driver.close(sessionId)
      sessionId = null
    }
    stopRef.current = stop

    const apply = (session: Live, m: FrameMessage) => {
      if (ended) return
      if (m.kind === 'clipboard' || m.kind === 'sync') {
        try {
          if (m.kind === 'sync') {
            // The session may already be gone.
            driver.ack(session.id).catch(() => {})
          } else {
            session.received += 1
            if (useSettingsStore.getState().settings.vncClipboardSync) {
              writeClipboard(session, m.text)
            } else {
              session.pending = null
            }
          }
        } catch (e) {
          // Reported, not thrown: the messages after this one still have to be applied.
          console.error(e)
        }
        return
      }
      if (m.kind === 'closed') {
        ended = true
        release()
        setState({ status: 'closed', reason: m.reason })
        return
      }
      try {
        switch (m.kind) {
          case 'rect':
            if (m.w > 0 && m.h > 0) ctx.putImageData(new ImageData(m.rgba, m.w, m.h), m.x, m.y)
            break
          case 'copy':
            ctx.drawImage(canvas, m.srcX, m.srcY, m.w, m.h, m.x, m.y, m.w, m.h)
            break
          case 'resize':
            canvas.width = m.w
            canvas.height = m.h
            break
          case 'cursor':
            if (m.w === 0 || m.h === 0) {
              canvas.style.cursor = 'none'
            } else if (m.w > 128 || m.h > 128) {
              // Browsers refuse cursor images larger than this.
              canvas.style.cursor = 'default'
            } else {
              const image = document.createElement('canvas')
              image.width = m.w
              image.height = m.h
              const imageCtx = image.getContext('2d')
              if (!imageCtx) throw new Error('no 2d context for the cursor')
              imageCtx.putImageData(new ImageData(m.rgba, m.w, m.h), 0, 0)
              canvas.style.cursor = `url(${image.toDataURL()}) ${m.hotX} ${m.hotY}, default`
            }
            break
        }
      } catch {
        stop()
        setState({ status: 'closed', reason: PICTURE_ERROR })
      }
    }

    driver
      .open((m) => {
        if (live) apply(live, m)
        else if (!ended) queue.push(m)
      })
      .then(
        (opened) => {
          if (ended) {
            driver.close(opened.id)
            return
          }
          sessionId = opened.id
          canvas.width = opened.width
          canvas.height = opened.height
          const session: Live = {
            id: opened.id,
            keyboard: new RemoteKeyboard(
              (down, keysym) => driver.key(opened.id, down, keysym),
              detectPlatform(navigator.userAgent),
            ),
            wheel: new WheelSteps(),
            buttons: 0,
            x: 0,
            y: 0,
            move: 0,
            synced: '',
            pending: null,
            received: 0,
          }
          live = session
          liveRef.current = session
          setState({ status: 'connected' })
          for (const m of queue.splice(0)) apply(session, m)
        },
        (e: unknown) => {
          if (ended) return
          ended = true
          const err = e as AppError
          if (err && typeof err === 'object' && 'kind' in err) {
            if (err.kind === 'hostKeyUnknown') {
              const prompt: HostKeyPrompt = { kind: 'unknown', ...err.message, certificate: true }
              setState({ status: 'trust', prompt })
              return
            }
            if (err.kind === 'hostKeyMismatch') {
              const prompt: HostKeyPrompt = { kind: 'mismatch', ...err.message, certificate: true }
              setState({ status: 'trust', prompt })
              return
            }
            const error =
              'message' in err && typeof err.message === 'string' ? err.message : err.kind
            setState({ status: 'failed', error })
            return
          }
          setState({ status: 'failed', error: String(e) })
        },
      )
  }, [driver])
  const connectRef = useRef(connect)

  useEffect(() => {
    connectRef.current = connect
    connect()
    return () => stopRef.current()
  }, [connect])

  const syncClipboard = useCallback(() => {
    const live = liveRef.current
    if (!live) return
    if (!useSettingsStore.getState().settings.vncClipboardSync) {
      live.pending = null
      return
    }
    const received = live.received
    navigator.clipboard.readText().then(
      (local) => {
        if (liveRef.current !== live || live.received !== received) return
        if (!useSettingsStore.getState().settings.vncClipboardSync) return
        if (local === '' || local === live.synced) {
          if (live.pending !== null) writeClipboard(live, live.pending)
          return
        }
        // Text copied here since the last sync is newer than server text that was never written.
        const fromServer = local === live.pending
        live.synced = local
        live.pending = null
        if (!fromServer) driver.clipboard(live.id, local)
      },
      // Reading is refused while the window is not focused.
      () => {},
    )
  }, [driver])

  useEffect(() => {
    if (active) {
      syncClipboard()
      return
    }
    const live = liveRef.current
    if (live) {
      releaseButtons(driver, live)
      live.keyboard.releaseAll()
    }
    canvasRef.current?.blur()
  }, [active, syncClipboard, driver])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const live = liveRef.current
      const point = pointOn(canvas, e)
      if (!live || !point) return
      live.x = point.x
      live.y = point.y
      for (const mask of live.wheel.push(e.deltaX, e.deltaY, e.deltaMode)) {
        live.keyboard.flush()
        sendPointer(driver, live, live.buttons | mask)
        sendPointer(driver, live, live.buttons)
      }
    }
    // React registers its own wheel listener as passive, which cannot prevent scrolling.
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => canvas.removeEventListener('wheel', onWheel)
  }, [driver])

  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === containerRef.current)
    document.addEventListener('fullscreenchange', onChange)
    return () => document.removeEventListener('fullscreenchange', onChange)
  }, [])

  useEffect(() => {
    if (!fullscreen || (state.status === 'connected' && active)) return
    // The browser refused; there is nothing to show.
    document.exitFullscreen().catch(() => {})
  }, [fullscreen, state.status, active])

  useEffect(() => {
    if (!active || (state.status !== 'closed' && state.status !== 'failed')) return
    const focused = document.activeElement
    if (!focused || focused === document.body || containerRef.current?.contains(focused)) {
      reconnectRef.current?.focus()
    }
  }, [state.status, active])

  const onPointer = (e: PointerEvent<HTMLCanvasElement>) => {
    const live = liveRef.current
    const point = pointOn(e.currentTarget, e)
    if (!live || !point) return
    if (e.type === 'pointerdown') {
      e.currentTarget.setPointerCapture(e.pointerId)
      e.currentTarget.focus()
    }
    live.x = point.x
    live.y = point.y
    const buttons = pointerButtons(e.buttons)
    if (e.type === 'pointermove' && buttons === live.buttons) {
      if (live.move === 0) {
        live.move = requestAnimationFrame(() => {
          live.move = 0
          driver.pointer(live.id, live.buttons, live.x, live.y)
        })
      }
      return
    }
    if (buttons !== live.buttons) live.keyboard.flush()
    live.buttons = buttons
    sendPointer(driver, live, buttons)
  }

  const onPointerLost = () => {
    const live = liveRef.current
    if (live) releaseButtons(driver, live)
  }

  const onKey = (e: KeyboardEvent<HTMLCanvasElement>) => {
    const keyboard = liveRef.current?.keyboard
    if (!keyboard || e.nativeEvent.isComposing) return
    const used =
      e.type === 'keydown' ? keyboard.keydown(e.nativeEvent) : keyboard.keyup(e.nativeEvent)
    if (!used) return
    e.preventDefault()
    e.stopPropagation()
  }

  const ctrlAltDel = () => {
    const live = liveRef.current
    if (!live) return
    for (const keysym of [0xffe3, 0xffe9, 0xffff]) driver.key(live.id, true, keysym)
    for (const keysym of [0xffff, 0xffe9, 0xffe3]) driver.key(live.id, false, keysym)
  }

  // The closing certificate dialog would otherwise still hold focus when the overlay appears.
  const leaveDialog = () => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
  }

  const message =
    state.status === 'closed' ? state.reason : state.status === 'failed' ? state.error : null

  return (
    <div ref={containerRef} className="flex h-full w-full flex-col bg-background">
      {state.status === 'connected' && (
        <div className="flex shrink-0 items-center gap-1 border-border border-b p-1">
          <button
            type="button"
            onClick={ctrlAltDel}
            className="rounded px-2 py-1 text-xs hover:bg-muted"
          >
            Ctrl+Alt+Del
          </button>
          <button
            type="button"
            onClick={connect}
            className="rounded px-2 py-1 text-xs hover:bg-muted"
          >
            Reconnect
          </button>
          <button
            type="button"
            onClick={() => {
              stopRef.current()
              setState({ status: 'closed', reason: 'Disconnected.' })
            }}
            className="rounded px-2 py-1 text-xs hover:bg-muted"
          >
            Disconnect
          </button>
          <button
            type="button"
            aria-pressed={fullscreen}
            onClick={() => {
              const change = fullscreen
                ? document.exitFullscreen()
                : containerRef.current?.requestFullscreen()
              // The browser refused; there is nothing to show.
              change?.catch(() => {})
            }}
            className="rounded px-2 py-1 text-xs hover:bg-muted"
          >
            {fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
          </button>
        </div>
      )}
      <div
        className={cn(
          'flex min-h-0 flex-1 items-center justify-center overflow-hidden border border-transparent bg-black focus-within:border-ring',
          state.status !== 'connected' && 'hidden',
        )}
      >
        <canvas
          ref={canvasRef}
          tabIndex={0}
          aria-label="Remote desktop"
          data-remote-desktop
          onPointerDown={onPointer}
          onPointerMove={onPointer}
          onPointerUp={onPointer}
          onPointerCancel={onPointerLost}
          onLostPointerCapture={onPointerLost}
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={onKey}
          onKeyUp={onKey}
          onFocus={syncClipboard}
          onBlur={() => {
            const live = liveRef.current
            if (!live) return
            releaseButtons(driver, live)
            live.keyboard.releaseAll()
          }}
          className="max-h-full max-w-full touch-none object-contain outline-none"
        />
      </div>
      {state.status !== 'connected' && (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 text-sm">
          {message !== null ? (
            <>
              <p
                className={cn(
                  'max-w-xs text-center',
                  state.status === 'failed' ? 'text-destructive' : 'text-muted-foreground',
                )}
              >
                {message}
              </p>
              <div className="flex gap-2">
                <button
                  ref={reconnectRef}
                  type="button"
                  onClick={connect}
                  className="rounded border border-border px-3 py-1.5 hover:bg-muted"
                >
                  Reconnect
                </button>
                <button
                  type="button"
                  onClick={() => removeTab(tabId)}
                  className="rounded border border-border px-3 py-1.5 hover:bg-muted"
                >
                  Close tab
                </button>
              </div>
            </>
          ) : state.status === 'connecting' ? (
            <p className="text-muted-foreground">Connecting…</p>
          ) : null}
        </div>
      )}
      <HostKeyDialog
        prompt={state.status === 'trust' ? state.prompt : null}
        onAccept={async () => {
          if (state.status !== 'trust') return
          const p = state.prompt
          const attempt = attemptRef.current
          leaveDialog()
          setState({ status: 'connecting' })
          try {
            await trustHostKey(p.host, p.port, p.kind === 'unknown' ? p.fingerprint : p.offered)
          } catch (e) {
            if (attemptRef.current !== attempt) return
            setState({ status: 'failed', error: e instanceof Error ? e.message : String(e) })
            return
          }
          connectRef.current()
        }}
        onReject={() => {
          leaveDialog()
          setState({ status: 'failed', error: 'Certificate rejected.' })
        }}
      />
    </div>
  )
}
