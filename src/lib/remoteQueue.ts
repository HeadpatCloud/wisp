// Input for one session that is not sent yet. `buttons` is the mask of the last pointer event
// made, and `move` says the last queued event is a pointer event that changed no button.
interface Queue<E> {
  events: E[]
  sent: (() => void)[]
  flight: Promise<void> | null
  buttons: number
  move: boolean
}

export function remoteQueue<E extends { kind: string; buttons?: number }>(
  send: (id: string, events: E[]) => Promise<unknown>,
) {
  // The backend runs every call as a task of its own, so input keeps its order only when one
  // call per session is under way at a time.
  const queues = new Map<string, Queue<E>>()

  const flush = (id: string, queue: Queue<E>) => {
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
    queue.flight = send(id, events).then(done, done)
  }

  const push = (id: string, event: E) => {
    let queue = queues.get(id)
    if (!queue) {
      queue = { events: [], sent: [], flight: null, buttons: 0, move: false }
      queues.set(id, queue)
    }
    const move = event.kind === 'pointer' && event.buttons === queue.buttons
    if (event.kind === 'pointer') queue.buttons = event.buttons ?? 0
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

  // Undefined when nothing was ever queued, so that a caller can go on without a turn of the
  // event loop in between.
  const drain = (id: string) => {
    const queue = queues.get(id)
    if (!queue) return undefined
    return (async () => {
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
    })()
  }

  return { push, drain }
}
