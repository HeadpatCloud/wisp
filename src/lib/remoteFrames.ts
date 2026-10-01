type Pixels = Uint8ClampedArray<ArrayBuffer>

export type FrameMessage =
  | { kind: 'rect'; x: number; y: number; w: number; h: number; rgba: Pixels }
  | { kind: 'copy'; x: number; y: number; w: number; h: number; srcX: number; srcY: number }
  | { kind: 'resize'; w: number; h: number }
  | { kind: 'cursor'; hotX: number; hotY: number; w: number; h: number; rgba: Pixels }
  | { kind: 'clipboard'; text: string }
  | { kind: 'closed'; reason: string }
  | { kind: 'sync' }

function invalid(): Error {
  return new Error('invalid frame message')
}

function fields(view: DataView, count: number): number[] {
  if (view.byteLength < 1 + count * 2) throw invalid()
  return Array.from({ length: count }, (_, i) => view.getUint16(1 + i * 2))
}

function pixels(buf: ArrayBuffer, w: number, h: number): Pixels {
  if (buf.byteLength !== 9 + w * h * 4) throw invalid()
  return new Uint8ClampedArray(buf, 9)
}

function text(buf: ArrayBuffer): string {
  return new TextDecoder('utf-8', { ignoreBOM: true }).decode(new Uint8Array(buf, 1))
}

// Where Tauri falls back to postMessage, a body of 1024 bytes or more arrives as a number array.
export function decodeFrame(buf: ArrayBuffer | number[]): FrameMessage {
  if (Array.isArray(buf)) return decodeFrame(new Uint8Array(buf).buffer)
  const view = new DataView(buf)
  if (view.byteLength === 0) throw invalid()
  switch (view.getUint8(0)) {
    case 1: {
      const [x, y, w, h] = fields(view, 4)
      return { kind: 'rect', x, y, w, h, rgba: pixels(buf, w, h) }
    }
    case 2: {
      const [x, y, w, h, srcX, srcY] = fields(view, 6)
      if (view.byteLength !== 13) throw invalid()
      return { kind: 'copy', x, y, w, h, srcX, srcY }
    }
    case 3: {
      const [w, h] = fields(view, 2)
      if (view.byteLength !== 5) throw invalid()
      return { kind: 'resize', w, h }
    }
    case 4: {
      const [hotX, hotY, w, h] = fields(view, 4)
      return { kind: 'cursor', hotX, hotY, w, h, rgba: pixels(buf, w, h) }
    }
    case 5:
      return { kind: 'clipboard', text: text(buf) }
    case 6:
      return { kind: 'closed', reason: text(buf) }
    case 7:
      if (view.byteLength !== 1) throw invalid()
      return { kind: 'sync' }
    default:
      throw invalid()
  }
}
