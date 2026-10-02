import { Channel } from '@tauri-apps/api/core'
import { commands, type FrameBytes } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import type { RemoteDriver } from '@/lib/remoteDriver'
import { decodeFrame, type FrameMessage } from '@/lib/remoteFrames'

export function vncDriver(target: {
  host: string
  port: number
  username: string | null
  secretId: string | null
}): RemoteDriver {
  const close = async (id: string) => {
    unwrap(await commands.vncClose(id))
  }
  return {
    async open(onFrame) {
      let id: string | null = null
      let unreadable = false
      const channel = new Channel<FrameBytes>()
      // The bindings say number array; what arrives is an ArrayBuffer, or a number array on a
      // webview without the custom-protocol IPC.
      channel.onmessage = (buf: ArrayBuffer | number[]) => {
        if (unreadable) return
        let message: FrameMessage
        try {
          message = decodeFrame(buf)
        } catch {
          unreadable = true
          onFrame({ kind: 'closed', reason: 'The server sent data this app could not read.' })
          if (id !== null) close(id)
          return
        }
        onFrame(message)
      }
      const res = await commands.vncOpen(
        target.host,
        target.port,
        target.username,
        target.secretId,
        channel,
      )
      // The error object itself, so that the view can tell a certificate prompt by its kind.
      if (res.status === 'error') throw res.error
      id = res.data.id
      if (unreadable) close(id)
      return res.data
    },
    async pointer(id, buttons, x, y) {
      unwrap(await commands.vncPointer(id, buttons, x, y))
    },
    async key(id, down, keysym) {
      unwrap(await commands.vncKey(id, down, keysym))
    },
    async clipboard(id, text) {
      unwrap(await commands.vncCutText(id, text))
    },
    async ack(id) {
      unwrap(await commands.vncAck(id))
    },
    close,
  }
}
