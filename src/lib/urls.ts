export interface RemoteOrigin {
  user: string
  host: string
  port: number
}

// Percent-encode each segment but keep the separators.
export function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/')
}

// A bare IPv6 literal has to be bracketed in a URL, or the colons read as the port.
export function urlHost(host: string): string {
  if (host.startsWith('[')) return host
  return host.includes(':') ? `[${host}]` : host
}

export function remoteUrl(scheme: string, o: RemoteOrigin, path: string): string {
  const p = path.startsWith('/') ? path : `/${path}`
  return `${scheme}://${encodeURIComponent(o.user)}@${urlHost(o.host)}:${o.port}${encodePath(p)}`
}
