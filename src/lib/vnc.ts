import { Channel } from '@tauri-apps/api/core'
import { commands, type FrameBytes, type VncInput } from '@/bindings'
import { unwrap } from '@/lib/ipc'
import type { RemoteDriver } from '@/lib/remoteDriver'
import { decodeFrame, type FrameMessage } from '@/lib/remoteFrames'

// Input for one session that is not sent yet. `buttons` is the mask of the last pointer event
// made, and `move` says the last queued event is a pointer event that changed no button.
interface Queue {
  events: VncInput[]
  sent: (() => void)[]
  flight: Promise<void> | null
  buttons: number
  move: boolean
}

export function vncDriver(target: {
  host: string
  port: number
  username: string | null
  secretId: string | null
  profileId: string | null
}): RemoteDriver {
  // The backend runs every call as a task of its own, so input keeps its order only when one
  // call per session is under way at a time.
  const queues = new Map<string, Queue>()

  const flush = (id: string, queue: Queue) => {
    const { events, sent } = queue
    queue.events = []
    queue.sent = []
    queue.move = false
    const done = () => {
      for (const resolve of sent) resolve()
      queue.flight = null
      if (queue.events.length > 0) flush(id, queue)
    }
    // A failed call counts as sent: the session is gone or the IPC failed, and the view can do
    // nothing about either.
    queue.flight = commands.vncInput(id, events).then(done, done)
  }

  const push = (id: string, event: VncInput) => {
    let queue = queues.get(id)
    if (!queue) {
      queue = { events: [], sent: [], flight: null, buttons: 0, move: false }
      queues.set(id, queue)
    }
    const move = event.kind === 'pointer' && event.buttons === queue.buttons
    if (event.kind === 'pointer') queue.buttons = event.buttons
    // Only a move takes the place of the move before it; where a button changed is kept.
    if (move && queue.move) queue.events[queue.events.length - 1] = event
    else queue.events.push(event)
    queue.move = move
    const { sent } = queue
    const done = new Promise<void>((resolve) => {
      sent.push(resolve)
    })
    if (!queue.flight) flush(id, queue)
    return done
  }

  const close = async (id: string) => {
    const queue = queues.get(id)
    if (queue) {
      // What is queued goes out as a call of its own once the one in flight has returned.
      const sent = (async () => {
        while (queue.flight) await queue.flight
      })()
      // A server that stopped reading never lets the call in flight return; closing ends it.
      let timer: ReturnType<typeof setTimeout> | undefined
      const stuck = new Promise<void>((resolve) => {
        timer = setTimeout(resolve, 1000)
      })
      await Promise.race([sent, stuck])
      clearTimeout(timer)
      queues.delete(id)
    }
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
    pointer: (id, buttons, x, y) => push(id, { kind: 'pointer', buttons, x, y }),
    key: (id, down, keysym) => push(id, { kind: 'key', down, keysym }),
    clipboard: (id, text) => push(id, { kind: 'clipboard', text }),
    async ack(id) {
      unwrap(await commands.vncAck(id))
    },
    close,
  }
}
