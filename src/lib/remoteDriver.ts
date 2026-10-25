import type { FrameMessage } from '@/lib/remoteFrames'

export interface RemoteSession {
  id: string
  width: number
  height: number
  name: string
}

// Device pixels; scale is a percentage.
export interface Viewport {
  width: number
  height: number
  scale: number
}

export interface LockKeys {
  caps: boolean
  num: boolean
  scroll: boolean
}

export interface RemoteDriver {
  // Keys go out by their code, so a key without a keysym is still sent.
  scancodes?: boolean
  // An abort while the open is pending gives the login up; the open then rejects.
  open(
    onFrame: (m: FrameMessage) => void,
    viewport: Viewport,
    signal?: AbortSignal,
  ): Promise<RemoteSession>
  pointer(id: string, buttons: number, x: number, y: number): Promise<void>
  key(id: string, down: boolean, keysym: number, code: string): Promise<void>
  clipboard(id: string, text: string): Promise<void>
  ack(id: string): Promise<void>
  close(id: string): Promise<void>
  resize?(id: string, viewport: Viewport): Promise<void>
  locks?(id: string, locks: LockKeys): Promise<void>
}
