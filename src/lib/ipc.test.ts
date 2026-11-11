import { describe, expect, it } from 'vitest'
import { connectErrorText, formatError, unwrap } from './ipc'

describe('ipc', () => {
  it('unwraps ok', () => {
    expect(unwrap({ status: 'ok', data: 42 })).toBe(42)
  })

  it('throws formatted error', () => {
    expect(() => unwrap({ status: 'error', error: { kind: 'notFound', message: 'p1' } })).toThrow(
      'notFound: p1',
    )
  })

  it('shows an invalid error as its own sentence', () => {
    const message = "The folder Work can't go inside its own subfolder."
    expect(formatError({ kind: 'invalid', message })).toBe(message)
  })

  it('formats a message-less error', () => {
    expect(formatError({ kind: 'crypto' })).toBe('crypto')
  })

  it('a_connect_error_names_the_key_and_where_to_fix_it', () => {
    const required = {
      kind: 'passphraseRequired',
      message: 'C:\\Users\\me\\.ssh\\id_ed25519',
    } as const
    expect(connectErrorText(required)).toBe(
      'id_ed25519 needs a passphrase. Save it in Keys & passwords.',
    )
    expect(connectErrorText(required, false)).toBe('id_ed25519 needs a passphrase.')
    expect(connectErrorText({ kind: 'wrongPassphrase' })).toBe(
      'The key passphrase is wrong. Update it in Keys & passwords.',
    )
    expect(connectErrorText({ kind: 'wrongPassphrase' }, false)).toBe(
      'The key passphrase is wrong.',
    )
    expect(connectErrorText({ kind: 'auth', message: 'no method left' })).toBe(
      'Authentication failed: no method left',
    )
    expect(connectErrorText({ kind: 'tunnel', message: '127.0.0.1:8080: in use' })).toBe(
      '127.0.0.1:8080: in use',
    )
    expect(connectErrorText({ kind: 'crypto' })).toBe('crypto')
    expect(connectErrorText(new Error('gone'))).toBe('gone')
  })
})
