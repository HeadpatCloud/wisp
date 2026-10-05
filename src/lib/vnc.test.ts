import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { VncInput } from '@/bindings'
import type { FrameMessage } from './remoteFrames'

const m = vi.hoisted(() => ({
  channels: [] as { onmessage: (buf: ArrayBuffer | number[]) => void }[],
  vncOpen: vi.fn(),
  vncInput: vi.fn(),
  vncAck: vi.fn(),
  vncClose: vi.fn(),
}))
vi.mock('@/bindings', () => ({
  commands: {
    vncOpen: m.vncOpen,
    vncInput: m.vncInput,
    vncAck: m.vncAck,
    vncClose: m.vncClose,
  },
}))
vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: (buf: ArrayBuffer | number[]) => void = () => {}
    constructor() {
      m.channels.push(this)
    }
  },
}))

import { vncDriver } from './vnc'

const RECT = [1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7, 255]
const UNREADABLE = { kind: 'closed', reason: 'The server sent data this app could not read.' }
const OK = { status: 'ok', data: null }
const opened = { id: 'v1', width: 4, height: 2, name: 'desk' }
const target = { host: 'h', port: 5901, username: 'faye', secretId: 's1', profileId: null }
const press = (keysym: number): VncInput => ({ kind: 'key', down: true, keysym })
const release = (keysym: number): VncInput => ({ kind: 'key', down: false, keysym })
const pointer = (buttons: number, x: number, y: number): VncInput => ({
  kind: 'pointer',
  buttons,
  x,
  y,
})

function buf(...bytes: number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer
}

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

interface Call {
  id: string
  events: VncInput[]
  resolve: (result: unknown) => void
  reject: (error: unknown) => void
}

// Every `vncInput` call stays open until the test settles it.
function held(): Call[] {
  const calls: Call[] = []
  m.vncInput.mockImplementation(
    (id: string, events: VncInput[]) =>
      new Promise((resolve, reject) => {
        calls.push({ id, events: [...events], resolve, reject })
      }),
  )
  return calls
}

async function open() {
  const frames: FrameMessage[] = []
  const driver = vncDriver(target)
  const session = await driver.open((f) => frames.push(f))
  return { driver, session, frames, send: m.channels[0].onmessage }
}

async function rejection(error: unknown): Promise<unknown> {
  m.vncOpen.mockResolvedValue({ status: 'error', error })
  return vncDriver(target)
    .open(() => {})
    .catch((e: unknown) => e)
}

beforeEach(() => {
  vi.resetAllMocks()
  m.channels.length = 0
  m.vncOpen.mockResolvedValue({ status: 'ok', data: opened })
  for (const command of [m.vncInput, m.vncAck, m.vncClose]) command.mockResolvedValue(OK)
})

afterEach(() => {
  vi.useRealTimers()
})

test('open connects to the target and forwards a decoded rect', async () => {
  const { session, frames, send } = await open()
  expect(session).toEqual(opened)
  expect(m.vncOpen).toHaveBeenCalledWith('h', 5901, 'faye', 's1', null, m.channels[0])
  send(buf(...RECT))
  expect(frames).toHaveLength(1)
  const [frame] = frames
  if (frame.kind !== 'rect') throw new Error(`forwarded ${frame.kind}`)
  expect([frame.x, frame.y, frame.w, frame.h]).toEqual([1, 2, 1, 1])
  expect(Array.from(frame.rgba)).toEqual([9, 8, 7, 255])
})

test('every open of a profile tab sends the profile id and no secret id', async () => {
  const driver = vncDriver({
    host: 'h',
    port: 5901,
    username: 'faye',
    secretId: null,
    profileId: 'vnc-1',
  })
  await driver.open(() => {})
  await driver.close('v1')
  await driver.open(() => {})
  expect(m.vncOpen.mock.calls).toEqual([
    ['h', 5901, 'faye', null, 'vnc-1', m.channels[0]],
    ['h', 5901, 'faye', null, 'vnc-1', m.channels[1]],
  ])
})

test('a profile that no longer exists is reported without the vnc prefix', async () => {
  const rejected = await rejection({
    kind: 'notFound',
    message: 'vnc: this profile no longer exists',
  })
  expect(rejected).toEqual({ kind: 'notFound', message: 'this profile no longer exists' })
})

test('a number-array message is decoded like an ArrayBuffer', async () => {
  const { frames, send } = await open()
  send(RECT)
  send(buf(...RECT))
  expect(frames).toHaveLength(2)
  expect(frames[0]).toEqual(frames[1])
  expect(frames[0].kind).toBe('rect')
})

test('messages that arrive before open resolves are forwarded at once', async () => {
  let seen: FrameMessage[] = []
  const frames: FrameMessage[] = []
  m.vncOpen.mockImplementation(async () => {
    m.channels[0].onmessage(buf(3, 0, 8, 0, 6))
    seen = [...frames]
    return { status: 'ok', data: opened }
  })
  await vncDriver(target).open((f) => frames.push(f))
  expect(seen).toEqual([{ kind: 'resize', w: 8, h: 6 }])
  expect(frames).toHaveLength(1)
})

test('the message of a failed open loses its vnc prefix', async () => {
  const rejected = await rejection({ kind: 'internal', message: 'vnc: Authentication failed' })
  expect(rejected).toEqual({ kind: 'internal', message: 'Authentication failed' })
})

test('a failed open rejects with every other error object as it is', async () => {
  const errors = [
    { kind: 'hostKeyUnknown', message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' } },
    { kind: 'hostKeyMismatch', message: { host: 'vnc/h', port: 5900, stored: 'a', offered: 'b' } },
    { kind: 'io', message: 'refused (vnc: 1)' },
    { kind: 'crypto' },
  ]
  for (const error of errors) {
    const before = structuredClone(error)
    expect(await rejection(error)).toBe(error)
    expect(error).toEqual(before)
  }
})

test('an undecodable message closes the session, then is reported, once', async () => {
  const order: string[] = []
  m.vncClose.mockImplementation(async () => {
    order.push('close')
    return OK
  })
  const driver = vncDriver(target)
  await driver.open((f) => order.push(f.kind === 'closed' ? f.reason : f.kind))
  const send = m.channels[0].onmessage
  send(buf(9))
  expect(order).toEqual(['close', UNREADABLE.reason])
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  send(buf(9))
  send(buf(...RECT))
  send(buf(6, 0x62, 0x79, 0x65))
  await tick()
  expect(order).toEqual(['close', UNREADABLE.reason])
})

test('a close that fails after an undecodable message is not left unhandled', async () => {
  const { frames, send } = await open()
  m.vncClose.mockRejectedValue(new Error('ipc'))
  send(buf(9))
  await tick()
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
})

test('an undecodable message before open resolves closes the session once it has an id', async () => {
  const frames: FrameMessage[] = []
  m.vncOpen.mockImplementation(async () => {
    m.channels[0].onmessage(buf(9))
    expect(m.vncClose).not.toHaveBeenCalled()
    return { status: 'ok', data: opened }
  })
  m.vncClose.mockRejectedValue(new Error('ipc'))
  const session = await vncDriver(target).open((f) => frames.push(f))
  expect(session).toEqual(opened)
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  m.channels[0].onmessage(buf(...RECT))
  await tick()
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
})

test('a failed open after an undecodable message closes nothing', async () => {
  const error = { kind: 'internal', message: 'wrong password' }
  m.vncOpen.mockImplementation(async () => {
    m.channels[0].onmessage(buf(9))
    return { status: 'error', error }
  })
  const rejected = await vncDriver(target)
    .open(() => {})
    .catch((e: unknown) => e)
  expect(rejected).toBe(error)
  expect(m.vncClose).not.toHaveBeenCalled()
})

test('input goes out as events of one command', async () => {
  const driver = vncDriver(target)
  await driver.pointer('v1', 5, 10, 20)
  await driver.key('v1', true, 0xffe1)
  await driver.clipboard('v1', 'héllo')
  expect(m.vncInput.mock.calls).toEqual([
    ['v1', [pointer(5, 10, 20)]],
    ['v1', [press(0xffe1)]],
    ['v1', [{ kind: 'clipboard', text: 'héllo' }]],
  ])
})

test('inputs made while a call is in flight go out as one further call, in order', async () => {
  const calls = held()
  const driver = vncDriver(target)
  const first = driver.key('v1', true, 0x61)
  expect(calls).toHaveLength(1)
  expect(calls[0]).toMatchObject({ id: 'v1', events: [press(0x61)] })
  const rest = Promise.all([
    driver.key('v1', false, 0x61),
    driver.pointer('v1', 1, 10, 20),
    driver.clipboard('v1', 'hi'),
  ])
  expect(await settled(first)).toBe(false)
  expect(calls).toHaveLength(1)

  calls[0].resolve(OK)
  await first
  expect(calls).toHaveLength(2)
  expect(calls[1]).toMatchObject({
    id: 'v1',
    events: [release(0x61), pointer(1, 10, 20), { kind: 'clipboard', text: 'hi' }],
  })
  expect(await settled(rest)).toBe(false)

  calls[1].resolve(OK)
  await rest
  await tick()
  expect(calls).toHaveLength(2)
})

test('pointer moves queued behind a call collapse to the last one', async () => {
  const calls = held()
  const driver = vncDriver(target)
  driver.key('v1', true, 0x61)
  const moves = Promise.all(Array.from({ length: 10 }, (_, x) => driver.pointer('v1', 0, x, 5)))
  calls[0].resolve(OK)
  await tick()
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([pointer(0, 9, 5)])
  calls[1].resolve(OK)
  await moves
})

test('a button change between moves keeps both sides and where it happened', async () => {
  const calls = held()
  const driver = vncDriver(target)
  driver.key('v1', true, 0x61)
  for (const x of [0, 1, 2]) driver.pointer('v1', 0, x, 1)
  for (const x of [3, 4, 5]) driver.pointer('v1', 1, x, 1)
  for (const x of [6, 7, 8]) driver.pointer('v1', 0, x, 1)
  calls[0].resolve(OK)
  await tick()
  expect(calls[1].events).toEqual([
    pointer(0, 2, 1),
    pointer(1, 3, 1),
    pointer(1, 5, 1),
    pointer(0, 6, 1),
    pointer(0, 8, 1),
  ])
})

test('moves on either side of another event are not merged', async () => {
  const calls = held()
  const driver = vncDriver(target)
  driver.key('v1', true, 0x61)
  driver.pointer('v1', 0, 1, 1)
  driver.key('v1', false, 0x61)
  driver.pointer('v1', 0, 2, 2)
  calls[0].resolve(OK)
  await tick()
  expect(calls[1].events).toEqual([pointer(0, 1, 1), release(0x61), pointer(0, 2, 2)])
})

test('a move is not merged into one that is already on its way', async () => {
  const calls = held()
  const driver = vncDriver(target)
  driver.pointer('v1', 0, 1, 1)
  driver.pointer('v1', 0, 2, 2)
  expect(calls[0].events).toEqual([pointer(0, 1, 1)])
  calls[0].resolve(OK)
  await tick()
  expect(calls[1].events).toEqual([pointer(0, 2, 2)])
})

test('close waits for the call in flight and sends what is queued before it closes', async () => {
  const calls = held()
  const order: string[] = []
  m.vncClose.mockImplementation(async () => {
    order.push('close')
    return OK
  })
  const driver = vncDriver(target)
  driver.key('v1', true, 0xffe1)
  const releases = Promise.all([driver.key('v1', false, 0xffe1), driver.key('v1', false, 0xffe3)])
  const closing = driver.close('v1')
  await tick()
  expect(calls).toHaveLength(1)
  expect(order).toEqual([])

  calls[0].resolve(OK)
  await tick()
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([release(0xffe1), release(0xffe3)])
  expect(order).toEqual([])

  calls[1].resolve(OK)
  await closing
  await releases
  expect(order).toEqual(['close'])
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  expect(calls).toHaveLength(2)
})

test('close stops waiting after a second for an input call that never returns', async () => {
  vi.useFakeTimers()
  const calls = held()
  const driver = vncDriver(target)
  driver.key('v1', true, 0x61)
  driver.key('v1', false, 0x61)
  const closing = driver.close('v1')
  await vi.advanceTimersByTimeAsync(999)
  expect(m.vncClose).not.toHaveBeenCalled()

  await vi.advanceTimersByTimeAsync(1)
  expect(m.vncClose.mock.calls).toEqual([['v1']])
  await closing
  expect(calls).toHaveLength(1)
})

test('a close that did not have to wait leaves no timer behind', async () => {
  vi.useFakeTimers()
  const driver = vncDriver(target)
  await driver.key('v1', true, 0x61)
  await driver.close('v1')
  expect(m.vncClose.mock.calls).toEqual([['v1']])
  expect(vi.getTimerCount()).toBe(0)
})

test('close without input goes straight to the command', async () => {
  await vncDriver(target).close('v1')
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  expect(m.vncInput).not.toHaveBeenCalled()
})

test('a failed input call is followed by the next batch and leaves no promise pending', async () => {
  const calls = held()
  const driver = vncDriver(target)
  const first = driver.key('v1', true, 0x61)
  const second = driver.key('v1', false, 0x61)
  calls[0].reject(new Error('ipc'))
  await first
  expect(calls).toHaveLength(2)
  expect(calls[1].events).toEqual([release(0x61)])

  const third = driver.key('v1', true, 0x62)
  calls[1].resolve({ status: 'error', error: { kind: 'io', message: 'broken pipe' } })
  await second
  expect(calls).toHaveLength(3)
  expect(calls[2].events).toEqual([press(0x62)])
  calls[2].resolve(OK)
  await third
})

test('two sessions do not wait for each other', async () => {
  const calls = held()
  const driver = vncDriver(target)
  const first = driver.key('v1', true, 0x61)
  const other = driver.key('v2', true, 0x62)
  expect(calls.map((call) => call.id)).toEqual(['v1', 'v2'])
  calls[1].resolve(OK)
  await other
  expect(await settled(first)).toBe(false)
  calls[0].resolve(OK)
  await first
})

test('ack goes out at once while an input call is in flight', async () => {
  const calls = held()
  const driver = vncDriver(target)
  driver.key('v1', true, 0x61)
  await driver.ack('v1')
  expect(m.vncAck).toHaveBeenCalledWith('v1')
  expect(calls).toHaveLength(1)
})
