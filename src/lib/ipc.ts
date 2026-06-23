import type { AppError } from '@/bindings'

export type IpcResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError }

export function formatError(error: AppError): string {
  return 'message' in error ? `${error.kind}: ${error.message}` : error.kind
}

export function unwrap<T>(res: IpcResult<T>): T {
  if (res.status === 'error') throw new Error(formatError(res.error))
  return res.data
}
