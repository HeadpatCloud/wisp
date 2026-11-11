import type { AppError } from '@/bindings'

export type IpcResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError }

export function formatError(error: AppError): string {
  // Invalid errors are written for the user already.
  if (error.kind === 'invalid') return error.message
  return 'message' in error ? `${error.kind}: ${error.message}` : error.kind
}

// A failed connection in words: the key named, and, for a saved connection, where to fix it.
export function connectErrorText(e: unknown, keysPage = true): string {
  if (e && typeof e === 'object' && 'kind' in e) {
    const err = e as AppError
    const fix = (what: string) => (keysPage ? ` ${what} in Keys & passwords.` : '')
    if (err.kind === 'passphraseRequired') {
      const key = err.message.split(/[\\/]/).pop() || 'This key'
      return `${key} needs a passphrase.${fix('Save it')}`
    }
    if (err.kind === 'wrongPassphrase') return `The key passphrase is wrong.${fix('Update it')}`
    if (err.kind === 'auth') return `Authentication failed: ${err.message}`
    if ('message' in err && typeof err.message === 'string') return err.message
    return formatError(err)
  }
  return e instanceof Error ? e.message : String(e)
}

export function unwrap<T>(res: IpcResult<T>): T {
  if (res.status === 'error') throw new Error(formatError(res.error))
  return res.data
}
