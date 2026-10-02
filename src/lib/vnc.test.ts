import { beforeEach, expect, test, vi } from 'vitest'
import type { FrameMessage } from './remoteFrames'

const m = vi.hoisted(() => ({
  channels: [] as { onmessage: (buf: ArrayBuffer | number[]) => void }[],
  vncOpen: vi.fn(),
  vncPointer: vi.fn(),
  vncKey: vi.fn(),
  vncCutText: vi.fn(),
  vncAck: vi.fn(),
  vncClose: vi.fn(),
}))
vi.mock('@/bindings', () => ({
  commands: {
    vncOpen: m.vncOpen,
    vncPointer: m.vncPointer,
    vncKey: m.vncKey,
    vncCutText: m.vncCutText,
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
const opened = { id: 'v1', width: 4, height: 2, name: 'desk' }
const target = { host: 'h', port: 5901, username: 'faye', secretId: 's1' }

function buf(...bytes: number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer
}

async function open() {
  const frames: FrameMessage[] = []
  const driver = vncDriver(target)
  const session = await driver.open((f) => frames.push(f))
  return { driver, session, frames, send: m.channels[0].onmessage }
}

beforeEach(() => {
  vi.resetAllMocks()
  m.channels.length = 0
  m.vncOpen.mockResolvedValue({ status: 'ok', data: opened })
  for (const command of [m.vncPointer, m.vncKey, m.vncCutText, m.vncAck, m.vncClose]) {
    command.mockResolvedValue({ status: 'ok', data: null })
  }
})

test('open connects to the target and forwards a decoded rect', async () => {
  const { session, frames, send } = await open()
  expect(session).toEqual(opened)
  expect(m.vncOpen).toHaveBeenCalledWith('h', 5901, 'faye', 's1', m.channels[0])
  send(buf(...RECT))
  expect(frames).toHaveLength(1)
  const [frame] = frames
  if (frame.kind !== 'rect') throw new Error(`forwarded ${frame.kind}`)
  expect([frame.x, frame.y, frame.w, frame.h]).toEqual([1, 2, 1, 1])
  expect(Array.from(frame.rgba)).toEqual([9, 8, 7, 255])
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

test('open rejects with the error object of the backend', async () => {
  const error = {
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  }
  m.vncOpen.mockResolvedValue({ status: 'error', error })
  const rejected = await vncDriver(target)
    .open(() => {})
    .catch((e: unknown) => e)
  expect(rejected).toBe(error)
})

test('an undecodable message is reported once and closes the session', async () => {
  const { frames, send } = await open()
  send(buf(9))
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  send(buf(9))
  send(buf(...RECT))
  send(buf(6, 0x62, 0x79, 0x65))
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
  const session = await vncDriver(target).open((f) => frames.push(f))
  expect(session).toEqual(opened)
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
  expect(m.vncClose).toHaveBeenCalledWith('v1')
  m.channels[0].onmessage(buf(...RECT))
  expect(frames).toEqual([UNREADABLE])
  expect(m.vncClose).toHaveBeenCalledTimes(1)
})

test('a failed open after an undecodable message closes nothing', async () => {
  const error = { kind: 'internal', message: 'vnc: wrong password' }
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

test('ack calls the command', async () => {
  await vncDriver(target).ack('v1')
  expect(m.vncAck).toHaveBeenCalledWith('v1')
})

test('input and close call their commands', async () => {
  const driver = vncDriver(target)
  await driver.pointer('v1', 5, 10, 20)
  await driver.key('v1', true, 0xffe1)
  await driver.clipboard('v1', 'héllo')
  await driver.close('v1')
  expect(m.vncPointer).toHaveBeenCalledWith('v1', 5, 10, 20)
  expect(m.vncKey).toHaveBeenCalledWith('v1', true, 0xffe1)
  expect(m.vncCutText).toHaveBeenCalledWith('v1', 'héllo')
  expect(m.vncClose).toHaveBeenCalledWith('v1')
})

test('a failed input command rejects with its error as text', async () => {
  m.vncKey.mockResolvedValue({ status: 'error', error: { kind: 'io', message: 'broken pipe' } })
  await expect(vncDriver(target).key('v1', true, 0x61)).rejects.toThrow('io: broken pipe')
})
