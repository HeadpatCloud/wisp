export type AddressType = 'ssh' | 'sftp' | 'ftp' | 'ftps' | 'rdp' | 'vnc' | 's3'

export interface ParsedAddress {
  type: AddressType
  host: string
  // Null only for S3 without a port; every other type gets its default.
  port: number | null
  user: string | null
  bucket: string | null
}

const DEFAULT_PORT: Record<Exclude<AddressType, 's3'>, number> = {
  ssh: 22,
  sftp: 22,
  ftp: 21,
  ftps: 21,
  rdp: 3389,
  vnc: 5900,
}

const WORD: Record<Exclude<AddressType, 's3'>, string> = {
  ssh: 'SSH',
  sftp: 'SFTP',
  ftp: 'FTP',
  ftps: 'FTPS',
  rdp: 'RDP',
  vnc: 'VNC',
}

const SCHEMES = new Set<string>(['ssh', 'sftp', 'ftp', 'ftps', 'rdp', 'vnc', 's3'])
const NAME = /^[A-Za-z0-9._-]+$/

// The URL parser writes an IPv6 address in its canonical form, as Rust's Ipv6Addr does.
function canonicalV6(host: string): string | null {
  if (!host.includes(':') || !/^[0-9A-Fa-f:.]+$/.test(host)) return null
  try {
    return new URL(`http://[${host}]`).hostname.slice(1, -1)
  } catch {
    return null
  }
}

function parsePort(text: string): number | null {
  if (!/^\d+$/.test(text)) return null
  const n = Number(text)
  return n >= 1 && n <= 65535 ? n : null
}

function hostAndPort(text: string): { host: string; port: number | null } | null {
  if (text.startsWith('[')) {
    const end = text.indexOf(']')
    const host = text.slice(1, end)
    if (end < 0 || canonicalV6(host) === null) return null
    const after = text.slice(end + 1)
    if (after === '') return { host, port: null }
    const port = after.startsWith(':') ? parsePort(after.slice(1)) : null
    return port === null ? null : { host, port }
  }
  const colons = text.split(':').length - 1
  if (colons >= 2) return canonicalV6(text) === null ? null : { host: text, port: null }
  const [host, portText] = text.split(':')
  if (!NAME.test(host)) return null
  if (portText === undefined) return { host, port: null }
  const port = parsePort(portText)
  return port === null ? null : { host, port }
}

export function parseAddress(text: string): ParsedAddress | null {
  const trimmed = text.trim()
  if (!trimmed || /\s/.test(trimmed)) return null
  let type: AddressType = 'ssh'
  let rest = trimmed
  const scheme = /^([A-Za-z][A-Za-z0-9+.-]*):\/\//.exec(trimmed)
  if (scheme) {
    const name = scheme[1].toLowerCase()
    if (!SCHEMES.has(name)) return null
    type = name as AddressType
    rest = trimmed.slice(scheme[0].length)
  }
  if (type === 's3') {
    const [endpoint, bucket = ''] = rest.replace(/\/+$/, '').split('/', 2)
    const hp = hostAndPort(endpoint)
    return hp && { type, host: hp.host, port: hp.port, user: null, bucket: bucket || null }
  }
  rest = rest.replace(/\/$/, '')
  if (rest.includes('/')) return null
  const at = rest.lastIndexOf('@')
  // A password pasted as user:password@host is dropped, never kept as part of the user.
  const user = at >= 0 ? rest.slice(0, at).split(':')[0] : null
  if (user === '') return null
  const hp = hostAndPort(rest.slice(at + 1))
  return hp && { type, host: hp.host, port: hp.port ?? DEFAULT_PORT[type], user, bucket: null }
}

export function describeAddress(p: ParsedAddress): string {
  if (p.type === 's3') {
    const endpoint = p.port === null ? p.host : `${p.host}:${p.port}`
    return p.bucket ? `S3 bucket ${p.bucket} at ${endpoint}` : `S3 at ${endpoint}`
  }
  return `${WORD[p.type]} to ${p.host}${p.user ? ` as ${p.user}` : ''}, port ${p.port}`
}

// One spelling per address, as Rust's net::address_key: IPv6 canonical, names lowercased
// without a trailing dot. Dotted IPv4 needs nothing.
export function addressKey(address: string): string {
  const bare = address.startsWith('[') && address.endsWith(']') ? address.slice(1, -1) : address
  const v6 = canonicalV6(bare)
  if (v6 !== null) return v6
  const name = address.toLowerCase()
  return name.endsWith('.') ? name.slice(0, -1) : name
}
