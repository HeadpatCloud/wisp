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
import { suspendHotkeys } from '@/lib/hotkeys'
import type { RemoteDriver } from '@/lib/remoteDriver'
import type { FrameMessage } from '@/lib/remoteFrames'
import { detectPlatform, RemoteKeyboard, WheelSteps } from '@/lib/remoteInput'
import { trustHostKey } from '@/lib/ssh'
import { cn } from '@/lib/utils'
import { vncButtonMask } from '@/lib/vnc'
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
  clipboard: string
}

const PICTURE_ERROR = 'The picture could not be updated.'

function pointOn(canvas: HTMLCanvasElement, e: { clientX: number; clientY: number }) {
  const rect = canvas.getBoundingClientRect()
  const x = Math.floor(((e.clientX - rect.left) / rect.width) * canvas.width)
  const y = Math.floor(((e.clientY - rect.top) / rect.height) * canvas.height)
  return {
    x: Math.min(Math.max(x, 0), canvas.width - 1),
    y: Math.min(Math.max(y, 0), canvas.height - 1),
  }
}

function sendPointer(driver: RemoteDriver, live: Live, buttons: number, x: number, y: number) {
  cancelAnimationFrame(live.move)
  live.move = 0
  driver.pointer(live.id, buttons, x, y)
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
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const liveRef = useRef<Live | null>(null)
  const stopRef = useRef(() => {})
  const focusedRef = useRef(false)
  const removeTab = useSessionStore((s) => s.removeTab)

  const connect = useCallback(() => {
    stopRef.current()
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
          case 'clipboard':
            if (!useSettingsStore.getState().settings.vncClipboardSync) break
            session.clipboard = m.text
            // Writing is refused while the window is not focused.
            navigator.clipboard.writeText(m.text).catch(() => {})
            break
          case 'sync':
            // The session may already be gone.
            driver.ack(session.id).catch(() => {})
            break
          case 'closed':
            ended = true
            release()
            setState({ status: 'closed', reason: m.reason })
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
            clipboard: '',
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

  useEffect(() => {
    connect()
    return () => {
      stopRef.current()
      if (focusedRef.current) suspendHotkeys(false)
    }
  }, [connect])

  const pushClipboard = useCallback(() => {
    const live = liveRef.current
    if (!live || !useSettingsStore.getState().settings.vncClipboardSync) return
    navigator.clipboard.readText().then(
      (text) => {
        if (liveRef.current !== live || text === live.clipboard) return
        live.clipboard = text
        driver.clipboard(live.id, text)
      },
      // Reading is refused while the window is not focused.
      () => {},
    )
  }, [driver])

  useEffect(() => {
    if (active) {
      pushClipboard()
      return
    }
    liveRef.current?.keyboard.releaseAll()
    canvasRef.current?.blur()
  }, [active, pushClipboard])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const live = liveRef.current
      if (!live) return
      const { x, y } = pointOn(canvas, e)
      for (const mask of live.wheel.push(e.deltaX, e.deltaY, e.deltaMode)) {
        live.keyboard.flush()
        sendPointer(driver, live, live.buttons | mask, x, y)
        sendPointer(driver, live, live.buttons, x, y)
      }
    }
    // React registers its own wheel listener as passive, which cannot prevent scrolling.
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => canvas.removeEventListener('wheel', onWheel)
  }, [driver])

  const onPointer = (e: PointerEvent<HTMLCanvasElement>) => {
    const live = liveRef.current
    if (!live) return
    if (e.type === 'pointerdown') {
      e.currentTarget.setPointerCapture(e.pointerId)
      e.currentTarget.focus()
    }
    const { x, y } = pointOn(e.currentTarget, e)
    const buttons = vncButtonMask(e.buttons)
    if (e.type === 'pointermove' && buttons === live.buttons) {
      live.x = x
      live.y = y
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
    sendPointer(driver, live, buttons, x, y)
  }

  const onKey = (e: KeyboardEvent<HTMLCanvasElement>) => {
    const keyboard = liveRef.current?.keyboard
    if (!keyboard) return
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
            onClick={() => {
              if (document.fullscreenElement) document.exitFullscreen()
              else containerRef.current?.requestFullscreen()
            }}
            className="rounded px-2 py-1 text-xs hover:bg-muted"
          >
            Fullscreen
          </button>
        </div>
      )}
      <div
        className={cn(
          'flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-black',
          state.status !== 'connected' && 'hidden',
        )}
      >
        <canvas
          ref={canvasRef}
          tabIndex={0}
          onPointerDown={onPointer}
          onPointerMove={onPointer}
          onPointerUp={onPointer}
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={onKey}
          onKeyUp={onKey}
          onFocus={() => {
            focusedRef.current = true
            suspendHotkeys(true)
            pushClipboard()
          }}
          onBlur={() => {
            focusedRef.current = false
            suspendHotkeys(false)
            liveRef.current?.keyboard.releaseAll()
          }}
          className="max-h-full max-w-full object-contain outline-none"
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
          setState({ status: 'connecting' })
          try {
            await trustHostKey(p.host, p.port, p.kind === 'unknown' ? p.fingerprint : p.offered)
          } catch (e) {
            setState({ status: 'failed', error: e instanceof Error ? e.message : String(e) })
            return
          }
          connect()
        }}
        onReject={() => setState({ status: 'failed', error: 'Certificate rejected.' })}
      />
    </div>
  )
}
