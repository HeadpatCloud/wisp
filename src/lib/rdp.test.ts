import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { RdpInput } from '@/bindings'
import type { FrameMessage } from './remoteFrames'

const m = vi.hoisted(() => ({
  channels: [] as { onmessage: (buf: ArrayBuffer | number[]) => void }[],
  rdpOpen: vi.fn(),
  rdpInput: vi.fn(),
  rdpResize: vi.fn(),
  rdpAck: vi.fn(),
  rdpClose: vi.fn(),
}))
vi.mock('@/bindings', () => ({
  commands: {
    rdpOpen: m.rdpOpen,
    rdpInput: m.rdpInput,
    rdpResize: m.rdpResize,
    rdpAck: m.rdpAck,
    rdpClose: m.rdpClose,
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

import { rdpDriver } from './rdp'
import { RemoteKeyboard } from './remoteInput'

const RECT = [1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7, 255]
const UNREADABLE = { kind: 'closed', reason: 'The server sent data this app could not read.' }
const OK = { status: 'ok', data: null }
const viewport = { width: 1280, height: 720, scale: 125 }
const opened = { id: 'r1', width: 1280, height: 720 }
const target = {
  host: 'h',
  port: 3389,
  username: 'faye',
  domain: null,
  secretId: 's1',
  profileId: null,
  keyboardLayout: null,
}
const press = (code: number, extended = false): RdpInput => ({
  kind: 'key',
  down: true,
  code,
  extended,
})
const release = (code: number, extended = false): RdpInput => ({
  kind: 'key',
  down: false,
  code,
  extended,
})

function buf(...bytes: number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0))

interface Call {
  id: string
  events: RdpInput[]
  resolve: (result: unknown) => void
}

// Every `rdpInput` call stays open until the test settles it.
function held(): Call[] {
  const calls: Call[] = []
  m.rdpInput.mockImplementation(
    (id: string, events: RdpInput[]) =>
      new Promise((resolve) => {
        calls.push({ id, events: [...events], resolve })
      }),
  )
  return calls
}

async function open() {
  const frames: FrameMessage[] = []
  const driver = rdpDriver(target)
  const session = await driver.open((f) => frames.push(f), viewport)
  return { driver, session, frames, send: m.channels[0].onmessage }
}

async function rejection(error: unknown): Promise<unknown> {
  m.rdpOpen.mockResolvedValue({ status: 'error', error })
  return rdpDriver(target)
    .open(() => {}, viewport)
    .catch((e: unknown) => e)
}

beforeEach(() => {
  vi.resetAllMocks()
  m.channels.length = 0
  m.rdpOpen.mockResolvedValue({ status: 'ok', data: opened })
  for (const command of [m.rdpInput, m.rdpResize, m.rdpAck, m.rdpClose]) {
    command.mockResolvedValue(OK)
  }
})

afterEach(() => {
  vi.useRealTimers()
})

test('open passes the target and the viewport and forwards a decoded rect', async () => {
  const { session, frames, send } = await open()
  expect(session).toEqual({ ...opened, name: 'h' })
  expect(m.rdpOpen).toHaveBeenCalledWith(
    expect.any(String),
    target,
    { width: 1280, height: 720, scale: 125 },
    m.channels[0],
  )
  send(buf(...RECT))
  expect(frames).toHaveLength(1)
  const [frame] = frames
  if (frame.kind !== 'rect') throw new Error(`forwarded ${frame.kind}`)
  expect([frame.x, frame.y, frame.w, frame.h]).toEqual([1, 2, 1, 1])
})

test('domain and keyboard layout are passed on', async () => {
  await rdpDriver({ ...target, domain: 'CORP', keyboardLayout: 0x407 }).open(() => {}, viewport)
  expect(m.rdpOpen.mock.calls[0][1]).toEqual({ ...target, domain: 'CORP', keyboardLayout: 0x407 })
})

test('a profile id is passed on, with no secret, on every connect', async () => {
  const driver = rdpDriver({ ...target, secretId: null, profileId: 'rdp-1' })
  await driver.open(() => {}, viewport)
  await driver.open(() => {}, viewport)
  expect(m.rdpOpen).toHaveBeenCalledTimes(2)
  for (const call of m.rdpOpen.mock.calls) {
    expect(call[1]).toMatchObject({ secretId: null, profileId: 'rdp-1' })
  }
  const [first, second] = m.rdpOpen.mock.calls.map((call) => call[0])
  expect(first).not.toBe(second)
})

test('a user name of only spaces is sent as empty', async () => {
  await rdpDriver({ ...target, username: '   ' }).open(() => {}, viewport)
  expect(m.rdpOpen.mock.calls[0][1].username).toBe('')
})

test('a user name is sent without the spaces around it', async () => {
  await rdpDriver({ ...target, username: '  CORP\\faye ' }).open(() => {}, viewport)
  expect(m.rdpOpen.mock.calls[0][1].username).toBe('CORP\\faye')
})

test('the message of a failed open loses its rdp prefix', async () => {
  const rejected = await rejection({ kind: 'internal', message: 'rdp: wrong password' })
  expect(rejected).toEqual({ kind: 'internal', message: 'wrong password' })
})

test('a message without the prefix is left alone', async () => {
  const error = { kind: 'io', message: 'refused (rdp: 1)' }
  expect(await rejection(error)).toBe(error)
})

test('a failed open rejects with every other error object as it is', async () => {
  const errors = [
    { kind: 'hostKeyUnknown', message: { host: 'rdp/h', port: 3389, fingerprint: 'SHA256:ab' } },
    { kind: 'hostKeyMismatch', message: { host: 'rdp/h', port: 3389, stored: 'a', offered: 'b' } },
    { kind: 'crypto' },
  ]
  for (const error of errors) {
    const before = structuredClone(error)
    expect(await rejection(error)).toBe(error)
    expect(error).toEqual(before)
  }
})

test('a failed open rejects with a string error as it is', async () => {
  const error = 'invalid args `port` for command `rdp_open`: integer `70000`, expected u16'
  expect(await rejection(error)).toBe(error)
})

test('an undecodable message closes the session, then is reported, once', async () => {
  const order: string[] = []
  m.rdpClose.mockImplementation(async () => {
    order.push('close')
    return OK
  })
  const driver = rdpDriver(target)
  await driver.open((f) => order.push(f.kind === 'closed' ? f.reason : f.kind), viewport)
  const send = m.channels[0].onmessage
  send(buf(9))
  expect(order).toEqual(['close', UNREADABLE.reason])
  expect(m.rdpClose).toHaveBeenCalledWith('r1')
  send(buf(9))
  send(buf(...RECT))
  await tick()
  expect(order).toEqual(['close', UNREADABLE.reason])
})

test('a close that fails after an undecodable message is not left unhandled', async () => {
  const { frames, send } = await open()
  m.rdpClose.mockRejectedValue(new Error('ipc'))
  send(buf(9))
  await tick()
  expect(frames).toEqual([UNREADABLE])
  expect(m.rdpClose).toHaveBeenCalledTimes(1)
})

test('an undecodable message before open resolves closes the session once it has an id', async () => {
  const frames: FrameMessage[] = []
  m.rdpOpen.mockImplementation(async () => {
    m.channels[0].onmessage(buf(9))
    expect(m.rdpClose).not.toHaveBeenCalled()
    return { status: 'ok', data: opened }
  })
  m.rdpClose.mockRejectedValue(new Error('ipc'))
  const session = await rdpDriver(target).open((f) => frames.push(f), viewport)
  expect(session).toEqual({ ...opened, name: 'h' })
  expect(frames).toEqual([UNREADABLE])
  expect(m.rdpClose).toHaveBeenCalledWith('r1')
  m.channels[0].onmessage(buf(...RECT))
  await tick()
  expect(frames).toEqual([UNREADABLE])
  expect(m.rdpClose).toHaveBeenCalledTimes(1)
})

test('a failed open after an undecodable message closes nothing', async () => {
  const error = { kind: 'internal', message: 'wrong password' }
  m.rdpOpen.mockImplementation(async () => {
    m.channels[0].onmessage(buf(9))
    return { status: 'error', error }
  })
  const rejected = await rdpDriver(target)
    .open(() => {}, viewport)
    .catch((e: unknown) => e)
  expect(rejected).toBe(error)
  expect(m.rdpClose).not.toHaveBeenCalled()
})

function pendingOpen() {
  let resolve: (result: unknown) => void = () => {}
  m.rdpOpen.mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done
      }),
  )
  const controller = new AbortController()
  const opening = rdpDriver(target)
    .open(() => {}, viewport, controller.signal)
    .catch((e: unknown) => ({ rejected: e }))
  return { controller, opening, resolve: (result: unknown) => resolve(result) }
}

test('aborting a pending open closes its id at once and the open is cancelled', async () => {
  const { controller, opening, resolve } = pendingOpen()
  controller.abort()

  expect(m.rdpClose.mock.calls).toEqual([[m.rdpOpen.mock.calls[0][0]]])
  resolve({ status: 'error', error: { kind: 'cancelled' } })
  expect(await opening).toEqual({ rejected: { kind: 'cancelled' } })
  expect(m.rdpClose).toHaveBeenCalledTimes(1)
})

test('an open that logged in just before its abort landed is cancelled, closed once', async () => {
  const { controller, opening, resolve } = pendingOpen()
  controller.abort()
  resolve({ status: 'ok', data: { ...opened, id: m.rdpOpen.mock.calls[0][0] } })

  expect(await opening).toEqual({ rejected: { kind: 'cancelled' } })
  expect(m.rdpClose).toHaveBeenCalledTimes(1)
})

test('an abort after the open resolved closes nothing', async () => {
  const controller = new AbortController()
  await rdpDriver(target).open(() => {}, viewport, controller.signal)
  controller.abort()

  expect(m.rdpClose).not.toHaveBeenCalled()
})

test('a key with a scancode goes out as a key event', async () => {
  const driver = rdpDriver(target)
  await driver.key('r1', true, 0x61, 'KeyA')
  await driver.key('r1', false, 0x61, 'KeyA')
  expect(m.rdpInput.mock.calls).toEqual([
    ['r1', [press(0x1e)]],
    ['r1', [release(0x1e)]],
  ])
})

test('an extended key is marked extended', async () => {
  await rdpDriver(target).key('r1', true, 0xff52, 'ArrowUp')
  expect(m.rdpInput.mock.calls).toEqual([['r1', [press(0x48, true)]]])
})

test('a key with no scancode and a character keysym goes out as unicode', async () => {
  const driver = rdpDriver(target)
  await driver.key('r1', true, 0xe9, '')
  await driver.key('r1', false, 0xe9, '')
  await driver.key('r1', true, 0x01000000 + 0x20ac, 'Unidentified')
  expect(m.rdpInput.mock.calls).toEqual([
    ['r1', [{ kind: 'unicode', down: true, codepoint: 233 }]],
    ['r1', [{ kind: 'unicode', down: false, codepoint: 233 }]],
    ['r1', [{ kind: 'unicode', down: true, codepoint: 0x20ac }]],
  ])
})

test('the keysym range that stands for a character has its edges', async () => {
  const driver = rdpDriver(target)
  for (const keysym of [0x1f, 0x7f, 0x80, 0x9f, 0x100, 0xffe1, 0x00ffffff, 0x01110000]) {
    await driver.key('r1', true, keysym, '')
  }
  expect(m.rdpInput).not.toHaveBeenCalled()
  for (const keysym of [0x20, 0x7e, 0xa0, 0xff]) await driver.key('r1', true, keysym, '')
  expect(m.rdpInput.mock.calls.map(([, events]) => events[0].kind)).toEqual([
    'unicode',
    'unicode',
    'unicode',
    'unicode',
  ])
})

test('a unicode keysym for a control character is dropped', async () => {
  const driver = rdpDriver(target)
  for (const keysym of [0x01000001, 0x01000009, 0x0100001f, 0x0100007f, 0x01000080, 0x0100009f]) {
    await driver.key('r1', true, keysym, '')
  }
  expect(m.rdpInput).not.toHaveBeenCalled()
  for (const keysym of [0x01000020, 0x0100007e, 0x010000a0])
    await driver.key('r1', true, keysym, '')
  expect(m.rdpInput.mock.calls.map(([, events]) => events[0])).toEqual([
    { kind: 'unicode', down: true, codepoint: 0x20 },
    { kind: 'unicode', down: true, codepoint: 0x7e },
    { kind: 'unicode', down: true, codepoint: 0xa0 },
  ])
})

test('a unicode keysym reaches past the basic plane up to the last code point', async () => {
  const driver = rdpDriver(target)
  await driver.key('r1', true, 0x0101f980, '')
  await driver.key('r1', true, 0x0110ffff, '')
  for (const keysym of [0x01110000, 0x01110001, 0x01ffffff])
    await driver.key('r1', true, keysym, '')
  expect(m.rdpInput.mock.calls).toEqual([
    ['r1', [{ kind: 'unicode', down: true, codepoint: 0x1f980 }]],
    ['r1', [{ kind: 'unicode', down: true, codepoint: 0x10ffff }]],
  ])
})

test('a key with no scancode and a non-character keysym is dropped', async () => {
  await rdpDriver(target).key('r1', true, 0xffe1, '')
  expect(m.rdpInput).not.toHaveBeenCalled()
})

test('a dropped key resolves at once even while a call is in flight', async () => {
  held()
  const driver = rdpDriver(target)
  driver.key('r1', true, 0x61, 'KeyA')
  await driver.key('r1', true, 0xffe1, '')
})

// What the view's keyboard sends through this driver, flattened over the calls.
async function typed(...steps: [boolean, string, string][]) {
  const driver = rdpDriver(target)
  const keyboard = new RemoteKeyboard(
    (down, keysym, code) => driver.key('r1', down, keysym, code),
    'windows',
    driver.scancodes === true,
  )
  for (const [down, code, key] of steps) {
    if (down) keyboard.keydown({ code, key })
    else keyboard.keyup({ code, key })
  }
  for (let n = 0; n < steps.length; n++) await tick()
  return m.rdpInput.mock.calls.flatMap(([, events]) => events)
}

test('a German dead key and the letter it composes go out as their scancodes', async () => {
  expect(
    await typed(
      [true, 'Equal', 'Dead'],
      [false, 'Equal', 'Dead'],
      [true, 'KeyE', 'é'],
      [false, 'KeyE', 'e'],
    ),
  ).toEqual([press(0x0d), release(0x0d), press(0x12), release(0x12)])
})

test('the German circumflex dead key goes out as its scancode', async () => {
  expect(
    await typed(
      [true, 'Backquote', 'Dead'],
      [false, 'Backquote', 'Dead'],
      [true, 'KeyE', 'ê'],
      [false, 'KeyE', 'e'],
    ),
  ).toEqual([press(0x29), release(0x29), press(0x12), release(0x12)])
})

test.each([
  ['Convert', 'Convert', 0x79],
  ['NonConvert', 'NonConvert', 0x7b],
  ['KanaMode', 'KanaMode', 0x70],
  ['Lang1', 'HangulMode', 0x72],
  ['Lang2', 'HanjaMode', 0x71],
  ['KeyA', 'Process', 0x1e],
])('the IME key %s with key %s goes out as scancode %i', async (code, key, scancode) => {
  expect(await typed([true, code, key], [false, code, key])).toEqual([
    press(scancode),
    release(scancode),
  ])
})

test('a dead key without a code is not sent', async () => {
  expect(await typed([true, '', 'Dead'], [false, '', 'Dead'])).toEqual([])
})

test('the pointer mask is passed whole', async () => {
  const driver = rdpDriver(target)
  await driver.pointer('r1', 0x180, 3, 4)
  await driver.pointer('r1', 0x181, 5, 6)
  expect(m.rdpInput.mock.calls).toEqual([
    ['r1', [{ kind: 'pointer', buttons: 0x180, x: 3, y: 4 }]],
    ['r1', [{ kind: 'pointer', buttons: 0x181, x: 5, y: 6 }]],
  ])
})

test('clipboard and lock keys go out as events', async () => {
  const driver = rdpDriver(target)
  await driver.clipboard('r1', 'héllo')
  await driver.locks?.('r1', { caps: true, num: false, scroll: true })
  expect(m.rdpInput.mock.calls).toEqual([
    ['r1', [{ kind: 'clipboard', text: 'héllo' }]],
    ['r1', [{ kind: 'locks', caps: true, num: false, scroll: true }]],
  ])
})

test('resize reaches its command with the viewport', async () => {
  await rdpDriver(target).resize?.('r1', { width: 1600, height: 900, scale: 150 })
  expect(m.rdpResize).toHaveBeenCalledWith('r1', 1600, 900, 150)
})

test('a failed resize is thrown', async () => {
  m.rdpResize.mockResolvedValue({ status: 'error', error: { kind: 'io', message: 'broken pipe' } })
  await expect(rdpDriver(target).resize?.('r1', viewport)).rejects.toThrow('broken pipe')
})

test('order across kinds is kept while a call is in flight', async () => {
  const calls = held()
  const driver = rdpDriver(target)
  driver.key('r1', true, 0x61, 'KeyA')
  const rest = Promise.all([
    driver.pointer('r1', 1, 10, 20),
    driver.key('r1', true, 0xe9, ''),
    driver.clipboard('r1', 'hi'),
    driver.locks?.('r1', { caps: true, num: false, scroll: false }),
    driver.key('r1', false, 0x61, 'KeyA'),
  ])
  expect(calls).toHaveLength(1)
  calls[0].resolve(OK)
  await tick()
  expect(calls[1].events).toEqual([
    { kind: 'pointer', buttons: 1, x: 10, y: 20 },
    { kind: 'unicode', down: true, codepoint: 233 },
    { kind: 'clipboard', text: 'hi' },
    { kind: 'locks', caps: true, num: false, scroll: false },
    release(0x1e),
  ])
  calls[1].resolve(OK)
  await rest
})

test('close sends the queued releases before it closes', async () => {
  const calls = held()
  const order: string[] = []
  m.rdpClose.mockImplementation(async () => {
    order.push('close')
    return OK
  })
  const driver = rdpDriver(target)
  driver.key('r1', true, 0xffe1, 'ShiftLeft')
  const releases = driver.key('r1', false, 0xffe1, 'ShiftLeft')
  const closing = driver.close('r1')
  await tick()
  expect(order).toEqual([])
  calls[0].resolve(OK)
  await tick()
  expect(calls[1].events).toEqual([release(0x2a)])
  expect(order).toEqual([])
  calls[1].resolve(OK)
  await closing
  await releases
  expect(order).toEqual(['close'])
  expect(m.rdpClose).toHaveBeenCalledWith('r1')
})

test('close without input goes straight to the command', async () => {
  await rdpDriver(target).close('r1')
  expect(m.rdpClose).toHaveBeenCalledWith('r1')
  expect(m.rdpInput).not.toHaveBeenCalled()
})

test('ack goes out at once while an input call is in flight', async () => {
  const calls = held()
  const driver = rdpDriver(target)
  driver.key('r1', true, 0x61, 'KeyA')
  await driver.ack('r1')
  expect(m.rdpAck).toHaveBeenCalledWith('r1')
  expect(calls).toHaveLength(1)
})
