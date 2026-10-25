import { afterEach, expect, test, vi } from 'vitest'
import { remoteQueue } from './remoteQueue'

type Ev = { kind: 'key'; n: number } | { kind: 'pointer'; buttons: number; x: number }

interface Call {
  id: string
  events: Ev[]
  resolve: (result?: unknown) => void
  reject: (error: unknown) => void
}

const key = (n: number): Ev => ({ kind: 'key', n })
const pointer = (buttons: number, x: number): Ev => ({ kind: 'pointer', buttons, x })
const tick = () => new Promise((resolve) => setTimeout(resolve, 0))

async function settled(promise: Promise<unknown>): Promise<boolean> {
  let done = false
  const mark = () => {
    done = true
  }
  promise.then(mark, mark)
  await tick()
  return done
}

// Every call stays open until the test settles it.
function held() {
  const calls: Call[] = []
  const queue = remoteQueue<Ev>(
    (id, events) =>
      new Promise((resolve, reject) => {
        calls.push({ id, events: [...events], resolve, reject })
      }),
  )
  return { calls, queue }
}

afterEach(() => {
  vi.useRealTimers()
})

test('the first event goes out at once as a call of its own', () => {
  const { calls, queue } = held()
  queue.push('s1', key(1))
  expect(calls).toHaveLength(1)
  expect(calls[0]).toMatchObject({ id: 's1', events: [key(1)] })
})

test('events made while a call is in flight go out as one further call, in order', async () => {
  const { calls, queue } = held()
  const first = queue.push('s1', key(1))
  const rest = Promise.all([
    queue.push('s1', key(2)),
    queue.push('s1', pointer(1, 10)),
    queue.push('s1', key(3)),
  ])
  expect(await settled(first)).toBe(false)
  expect(calls).toHaveLength(1)

  calls[0].resolve()
  await first
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([key(2), pointer(1, 10), key(3)])
  expect(await settled(rest)).toBe(false)

  calls[1].resolve()
  await rest
  await tick()
  expect(calls).toHaveLength(2)
})

test('pointer moves queued behind a call collapse to the last one', async () => {
  const { calls, queue } = held()
  queue.push('s1', key(1))
  const moves = Promise.all(Array.from({ length: 10 }, (_, x) => queue.push('s1', pointer(0, x))))
  calls[0].resolve()
  await tick()
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([pointer(0, 9)])
  calls[1].resolve()
  await moves
})

test('a button change between moves keeps both sides and where it happened', async () => {
  const { calls, queue } = held()
  queue.push('s1', key(1))
  for (const x of [0, 1, 2]) queue.push('s1', pointer(0, x))
  for (const x of [3, 4, 5]) queue.push('s1', pointer(1, x))
  for (const x of [6, 7, 8]) queue.push('s1', pointer(0, x))
  calls[0].resolve()
  await tick()
  expect(calls[1].events).toEqual([
    pointer(0, 2),
    pointer(1, 3),
    pointer(1, 5),
    pointer(0, 6),
    pointer(0, 8),
  ])
})

test('moves on either side of another event are not merged', async () => {
  const { calls, queue } = held()
  queue.push('s1', key(1))
  queue.push('s1', pointer(0, 1))
  queue.push('s1', key(2))
  queue.push('s1', pointer(0, 2))
  calls[0].resolve()
  await tick()
  expect(calls[1].events).toEqual([pointer(0, 1), key(2), pointer(0, 2)])
})

test('a move is not merged into one that is already on its way', async () => {
  const { calls, queue } = held()
  queue.push('s1', pointer(0, 1))
  queue.push('s1', pointer(0, 2))
  expect(calls[0].events).toEqual([pointer(0, 1)])
  calls[0].resolve()
  await tick()
  expect(calls[1].events).toEqual([pointer(0, 2)])
})

test('a failed call resolves its callers and the next batch goes out', async () => {
  const { calls, queue } = held()
  const first = queue.push('s1', key(1))
  const second = queue.push('s1', key(2))
  calls[0].reject(new Error('ipc'))
  await first
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([key(2)])

  const third = queue.push('s1', key(3))
  calls[1].resolve({ status: 'error' })
  await second
  expect(calls).toHaveLength(3)
  expect(calls[2].events).toEqual([key(3)])
  calls[2].resolve()
  await third
})

test('two sessions do not wait for each other', async () => {
  const { calls, queue } = held()
  const first = queue.push('s1', key(1))
  const other = queue.push('s2', key(2))
  expect(calls.map((call) => call.id)).toEqual(['s1', 's2'])
  calls[1].resolve()
  await other
  expect(await settled(first)).toBe(false)
  calls[0].resolve()
  await first
})

test('drain waits for the call in flight and what is queued behind it', async () => {
  const { calls, queue } = held()
  queue.push('s1', key(1))
  const releases = Promise.all([queue.push('s1', key(2)), queue.push('s1', key(3))])
  const draining = queue.drain('s1') as Promise<void>
  await tick()
  expect(calls).toHaveLength(1)
  expect(await settled(draining)).toBe(false)

  calls[0].resolve()
  await tick()
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([key(2), key(3)])
  expect(await settled(draining)).toBe(false)

  calls[1].resolve()
  await draining
  await releases
})

test('drain stops waiting after a second for a call that never returns', async () => {
  vi.useFakeTimers()
  const { calls, queue } = held()
  queue.push('s1', key(1))
  queue.push('s1', key(2))
  let done = false
  const draining = (queue.drain('s1') as Promise<void>).then(() => {
    done = true
  })
  await vi.advanceTimersByTimeAsync(999)
  expect(done).toBe(false)
  await vi.advanceTimersByTimeAsync(1)
  await draining
  expect(done).toBe(true)
  expect(calls).toHaveLength(1)
})

test('a drain that did not have to wait leaves no timer behind', async () => {
  vi.useFakeTimers()
  const queue = remoteQueue<Ev>(async () => {})
  await queue.push('s1', key(1))
  await queue.drain('s1')
  expect(vi.getTimerCount()).toBe(0)
})

test('drain for a session that never had input has nothing to wait for', () => {
  const send = vi.fn()
  expect(remoteQueue<Ev>(send).drain('s1')).toBeUndefined()
  expect(send).not.toHaveBeenCalled()
})
