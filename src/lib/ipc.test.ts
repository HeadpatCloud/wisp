import { describe, expect, it } from 'vitest'
import { formatError, unwrap } from './ipc'

describe('ipc', () => {
  it('unwraps ok', () => {
    expect(unwrap({ status: 'ok', data: 42 })).toBe(42)
  })

  it('throws formatted error', () => {
    expect(() => unwrap({ status: 'error', error: { kind: 'notFound', message: 'p1' } })).toThrow(
      'notFound: p1',
    )
  })

  it('formats a message-less error', () => {
    expect(formatError({ kind: 'crypto' })).toBe('crypto')
  })
})
