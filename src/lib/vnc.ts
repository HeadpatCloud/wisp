import { Channel } from '@tauri-apps/api/core'
import { commands, type FrameBytes, type VncInput } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import type { RemoteDriver } from '@/lib/remoteDriver'
import { decodeFrame, type FrameMessage } from '@/lib/remoteFrames'
import { remoteQueue } from '@/lib/remoteQueue'

export function vncDriver(target: {
  host: string
  port: number
  username: string | null
  secretId: string | null
  profileId: string | null
}): RemoteDriver {
  const queue = remoteQueue<VncInput>((id, events) => commands.vncInput(id, events))

  const close = async (id: string) => {
    const draining = queue.drain(id)
    if (draining) await draining
    unwrap(await commands.vncClose(id))
  }

  // For a session the view is told is over: a close that fails changes nothing for it.
  const abandon = (id: string) => {
    close(id).catch(() => {})
  }

  return {
    // `onFrame` must not throw: Tauri's channel delivers nothing more after a handler that did.
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
          if (id !== null) abandon(id)
          onFrame({ kind: 'closed', reason: 'The server sent data this app could not read.' })
          return
        }
        onFrame(message)
      }
      const res = await commands.vncOpen(
        target.host,
        target.port,
        target.username,
        target.secretId,
        target.profileId,
        channel,
      )
      if (res.status === 'error') {
        const { error } = res
        // The error object itself, so that the view can tell a certificate prompt by its kind.
        // It shows a message as it is, so the backend's own prefix is taken off.
        if (
          typeof error === 'object' &&
          'message' in error &&
          typeof error.message === 'string' &&
          error.message.startsWith('vnc: ')
        ) {
          throw { ...error, message: error.message.slice('vnc: '.length) }
        }
        throw error
      }
      id = res.data.id
      if (unreadable) abandon(id)
      return res.data
    },
    pointer: (id, buttons, x, y) =>
      queue.push(id, { kind: 'pointer', buttons: buttons & 0x7f, x, y }),
    key: (id, down, keysym) => queue.push(id, { kind: 'key', down, keysym }),
    clipboard: (id, text) => queue.push(id, { kind: 'clipboard', text }),
    async ack(id) {
      unwrap(await commands.vncAck(id))
    },
    close,
  }
}
