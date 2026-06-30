// Global concurrency limiter for file transfers (uploads + downloads, all panels). Runs up to
// `limit` at once and queues the rest; the limit is user-configurable via settings.
let limit = 3
let running = 0
const queue: Array<() => void> = []
const cancelledWhileQueued = new Set<string>()

export function setTransferLimit(n: number): void {
  limit = Math.min(16, Math.max(1, Math.floor(n) || 1))
  pump()
}

// Run a transfer under the global limit. Resolves when it finishes, or immediately if it was
// cancelled while still queued (in which case `task` never runs).
export function runTransfer(id: string, task: () => Promise<void>): Promise<void> {
  return new Promise((resolve) => {
    queue.push(() => {
      if (cancelledWhileQueued.delete(id)) {
        resolve()
        return
      }
      running++
      task()
        .catch(() => {})
        .finally(() => {
          running--
          resolve()
          pump()
        })
    })
    pump()
  })
}

// Skip a transfer that hasn't started yet, so it never runs.
export function cancelQueuedTransfer(id: string): void {
  cancelledWhileQueued.add(id)
}

function pump(): void {
  while (running < limit && queue.length > 0) {
    queue.shift()?.()
  }
}
