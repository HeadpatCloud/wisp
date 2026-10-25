import { Channel } from '@tauri-apps/api/core'
import { type AppError, commands, type FrameBytes, type RdpInput } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import type { RemoteDriver } from '@/lib/remoteDriver'
import { decodeFrame, type FrameMessage } from '@/lib/remoteFrames'
import { remoteQueue } from '@/lib/remoteQueue'
import { scancodeFor } from '@/lib/scancodes'

const UNICODE_KEYSYM = 0x01000000

// The keysyms that stand for one character: Latin-1 and the 0x01000000 + code point form, both
// without the control ranges.
function characterOf(keysym: number): number | null {
  const unicode = keysym > UNICODE_KEYSYM && keysym <= UNICODE_KEYSYM + 0x10ffff
  const codepoint = unicode ? keysym - UNICODE_KEYSYM : keysym
  if (!unicode && codepoint > 0xff) return null
  if (codepoint < 0x20 || (codepoint >= 0x7f && codepoint <= 0x9f)) return null
  return codepoint
}

export function rdpDriver(target: {
  host: string
  port: number
  username: string
  domain: string | null
  secretId: string | null
  profileId: string | null
  keyboardLayout: number | null
}): RemoteDriver {
  const queue = remoteQueue<RdpInput>((id, events) => commands.rdpInput(id, events))

  const close = async (id: string) => {
    const draining = queue.drain(id)
    if (draining) await draining
    unwrap(await commands.rdpClose(id))
  }

  // For a session the view is told is over: a close that fails changes nothing for it.
  const abandon = (id: string) => {
    close(id).catch(() => {})
  }

  return {
    scancodes: true,
    // `onFrame` must not throw: Tauri's channel delivers nothing more after a handler that did.
    async open(onFrame, viewport, signal) {
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
      const requested = crypto.randomUUID()
      const cancel = () => abandon(requested)
      signal?.addEventListener('abort', cancel)
      const res = await commands.rdpOpen(
        requested,
        { ...target, username: target.username.trim() },
        { width: viewport.width, height: viewport.height, scale: viewport.scale },
        channel,
      )
      signal?.removeEventListener('abort', cancel)
      // The close already sent ends a session that finished logging in all the same.
      if (signal?.aborted) throw { kind: 'cancelled' } satisfies AppError
      if (res.status === 'error') {
        const { error } = res
        // The error object itself, so that the view can tell a certificate prompt by its kind.
        // It shows a message as it is, so the backend's own prefix is taken off.
        if (
          typeof error === 'object' &&
          'message' in error &&
          typeof error.message === 'string' &&
          error.message.startsWith('rdp: ')
        ) {
          throw { ...error, message: error.message.slice('rdp: '.length) }
        }
        throw error
      }
      id = res.data.id
      if (unreadable) abandon(id)
      return { ...res.data, name: target.host }
    },
    pointer: (id, buttons, x, y) => queue.push(id, { kind: 'pointer', buttons, x, y }),
    key(id, down, keysym, code) {
      const scancode = scancodeFor(code)
      if (scancode) return queue.push(id, { kind: 'key', down, ...scancode })
      const codepoint = characterOf(keysym)
      if (codepoint === null) return Promise.resolve()
      return queue.push(id, { kind: 'unicode', down, codepoint })
    },
    clipboard: (id, text) => queue.push(id, { kind: 'clipboard', text }),
    locks: (id, locks) => queue.push(id, { kind: 'locks', ...locks }),
    async resize(id, viewport) {
      unwrap(await commands.rdpResize(id, viewport.width, viewport.height, viewport.scale))
    },
    async ack(id) {
      unwrap(await commands.rdpAck(id))
    },
    close,
  }
}
