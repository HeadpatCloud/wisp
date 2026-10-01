import type { FrameMessage } from '@/lib/remoteFrames'

export interface RemoteSession {
  id: string
  width: number
  height: number
  name: string
}

export interface RemoteDriver {
  open(onFrame: (m: FrameMessage) => void): Promise<RemoteSession>
  pointer(id: string, buttons: number, x: number, y: number): Promise<void>
  key(id: string, down: boolean, keysym: number): Promise<void>
  clipboard(id: string, text: string): Promise<void>
  ack(id: string): Promise<void>
  close(id: string): Promise<void>
}
