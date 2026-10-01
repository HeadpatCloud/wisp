import { describe, expect, it } from 'vitest'
import { decodeFrame } from './remoteFrames'

function buf(...bytes: number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer
}

describe('decodeFrame', () => {
  it('decodes a rect with a pixel view into the buffer', () => {
    const input = buf(1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7, 255)
    const m = decodeFrame(input)
    if (m.kind !== 'rect') throw new Error(`decoded ${m.kind}`)
    expect([m.x, m.y, m.w, m.h]).toEqual([1, 2, 1, 1])
    expect(m.rgba).toHaveLength(4)
    expect(Array.from(m.rgba)).toEqual([9, 8, 7, 255])
    expect(m.rgba.buffer).toBe(input)
  })

  it('decodes a copy', () => {
    expect(decodeFrame(buf(2, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6))).toEqual({
      kind: 'copy',
      x: 1,
      y: 2,
      w: 3,
      h: 4,
      srcX: 5,
      srcY: 6,
    })
  })

  it('decodes a resize', () => {
    expect(decodeFrame(buf(3, 7, 0x80, 4, 0x38))).toEqual({ kind: 'resize', w: 1920, h: 1080 })
  })

  it('decodes a cursor with a pixel view into the buffer', () => {
    const input = buf(4, 0, 1, 0, 2, 0, 1, 0, 1, 1, 2, 3, 4)
    const m = decodeFrame(input)
    if (m.kind !== 'cursor') throw new Error(`decoded ${m.kind}`)
    expect([m.hotX, m.hotY, m.w, m.h]).toEqual([1, 2, 1, 1])
    expect(m.rgba).toHaveLength(4)
    expect(Array.from(m.rgba)).toEqual([1, 2, 3, 4])
    expect(m.rgba.buffer).toBe(input)
  })

  it('decodes clipboard text as UTF-8', () => {
    expect(decodeFrame(buf(5, 0x68, 0xc3, 0xa9))).toEqual({ kind: 'clipboard', text: 'hé' })
  })

  it('decodes empty clipboard text', () => {
    expect(decodeFrame(buf(5))).toEqual({ kind: 'clipboard', text: '' })
  })

  it('decodes a closed reason', () => {
    expect(decodeFrame(buf(6, 0x62, 0x79, 0x65))).toEqual({ kind: 'closed', reason: 'bye' })
  })

  it.each([
    ['an empty buffer', buf()],
    ['an unknown type', buf(9)],
    ['a rect cut off inside its header', buf(1, 0, 1)],
    ['a rect with one pixel byte missing', buf(1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7)],
    ['a rect with trailing bytes', buf(1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7, 255, 0)],
    ['a copy one byte short', buf(2, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0)],
    ['a resize with a trailing byte', buf(3, 7, 0x80, 4, 0x38, 0)],
  ])('rejects %s', (_, input) => {
    expect(() => decodeFrame(input)).toThrow('invalid frame message')
  })
})
