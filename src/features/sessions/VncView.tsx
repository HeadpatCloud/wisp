import { type MouseEvent as ReactMouseEvent, useEffect, useRef, useState } from 'react'
import type { FrameUpdate } from '@/bindings'
import {
  keysymFor,
  openVnc,
  vncButtonMask,
  vncClose,
  vncCutText,
  vncKey,
  vncPointer,
} from '@/lib/vnc'

function apply(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement, frame: FrameUpdate) {
  if (frame.kind === 'raw') {
    const bytes = Uint8ClampedArray.from(atob(frame.data), (c) => c.charCodeAt(0))
    ctx.putImageData(new ImageData(bytes, frame.w, frame.h), frame.x, frame.y)
  } else {
    ctx.drawImage(
      canvas,
      frame.src_x,
      frame.src_y,
      frame.w,
      frame.h,
      frame.x,
      frame.y,
      frame.w,
      frame.h,
    )
  }
}

export function VncView({
  host,
  port,
  secretId,
}: {
  host: string
  port: number
  secretId: string | null
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const idRef = useRef<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let disposed = false
    const ctxRef = { current: null as CanvasRenderingContext2D | null }
    const pending: FrameUpdate[] = []
    openVnc(host, port, secretId, (frame) => {
      const canvas = canvasRef.current
      if (ctxRef.current && canvas) apply(ctxRef.current, canvas, frame)
      else pending.push(frame)
    })
      .then((opened) => {
        if (disposed) {
          vncClose(opened.id)
          return
        }
        idRef.current = opened.id
        const canvas = canvasRef.current
        if (!canvas) return
        canvas.width = opened.width
        canvas.height = opened.height
        const ctx = canvas.getContext('2d')
        ctxRef.current = ctx
        if (ctx) for (const f of pending) apply(ctx, canvas, f)
        pending.length = 0
      })
      .catch((e) => setError(String(e)))
    return () => {
      disposed = true
      if (idRef.current) vncClose(idRef.current)
    }
  }, [host, port, secretId])

  const onMouse = (e: ReactMouseEvent) => {
    const id = idRef.current
    const canvas = canvasRef.current
    if (!id || !canvas) return
    const rect = canvas.getBoundingClientRect()
    const x = Math.round(((e.clientX - rect.left) / rect.width) * canvas.width)
    const y = Math.round(((e.clientY - rect.top) / rect.height) * canvas.height)
    vncPointer(id, vncButtonMask(e.buttons), x, y)
  }

  if (error) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-black">
        <p className="text-destructive text-sm">{error}</p>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full items-center justify-center overflow-auto bg-black">
      <canvas
        ref={canvasRef}
        tabIndex={0}
        onMouseDown={onMouse}
        onMouseUp={onMouse}
        onMouseMove={onMouse}
        onContextMenu={(e) => e.preventDefault()}
        onFocus={async () => {
          const id = idRef.current
          if (!id) return
          try {
            const text = await navigator.clipboard.readText()
            if (text) vncCutText(id, text)
          } catch {}
        }}
        onKeyDown={(e) => {
          const id = idRef.current
          const sym = keysymFor(e.key)
          if (!id || sym === null) return
          e.preventDefault()
          vncKey(id, true, sym)
        }}
        onKeyUp={(e) => {
          const id = idRef.current
          const sym = keysymFor(e.key)
          if (!id || sym === null) return
          e.preventDefault()
          vncKey(id, false, sym)
        }}
        className="max-h-full max-w-full outline-none"
      />
    </div>
  )
}
