export interface RemoteOrigin {
  user: string
  host: string
  port: number
}

// Percent-encode each segment but keep the separators.
export function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/')
}

export function remoteUrl(scheme: string, o: RemoteOrigin, path: string): string {
  const p = path.startsWith('/') ? path : `/${path}`
  return `${scheme}://${encodeURIComponent(o.user)}@${o.host}:${o.port}${encodePath(p)}`
}
