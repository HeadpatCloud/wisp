import type {
  ConnAppearance,
  Connection,
  Folder,
  Host,
  Login,
  SavedLogin,
  ShellInfo,
  Tag,
  Tunnel,
} from '@/bindings'

export type ConnectionType = Connection['kind']['type']

export const THIS_COMPUTER = 'this-computer'

const TYPE_NAME: Record<ConnectionType, string> = {
  ssh: 'SSH',
  sftp: 'Files',
  ftp: 'FTP',
  s3: 'S3',
  rdp: 'RDP',
  vnc: 'VNC',
  local: 'Local shell',
}

export function typeName(t: ConnectionType): string {
  return TYPE_NAME[t]
}

export function effectiveAddress(host: Host, c: Connection): string {
  return c.address ?? host.address
}

export function loginUser(login: Login, logins: SavedLogin[]): string | null {
  if (login.kind === 'own') return login.username || null
  return logins.find((l) => l.id === login.loginId)?.username ?? null
}

function shellName(program: string, shells: ShellInfo[]): string {
  const known = shells.find((s) => s.program === program)
  if (known) return known.name
  return program.split(/[\\/]/).pop() ?? program
}

export function connectionLabel(
  _host: Host,
  c: Connection,
  logins: SavedLogin[],
  shells: ShellInfo[] = [],
): string {
  const k = c.kind
  if (k.type === 'local') {
    const name = c.label || shellName(k.program, shells)
    return k.startFolder ? `${name} · ${k.startFolder}` : name
  }
  if (c.label) return `${TYPE_NAME[k.type]} · ${c.label}`
  if (k.type === 's3') return k.bucket ? `Bucket ${k.bucket}` : 'S3'
  if (k.type === 'ssh' || k.type === 'rdp') {
    const user = loginUser(k.login, logins)
    if (user) return `${TYPE_NAME[k.type]} · ${user}`
  }
  return TYPE_NAME[k.type]
}

export function findConnection(
  hosts: Host[],
  id: string,
): { host: Host; connection: Connection } | null {
  for (const host of hosts) {
    const connection = host.connections.find((c) => c.id === id)
    if (connection) return { host, connection }
  }
  return null
}

export function folderPath(folderId: string | null, folders: Folder[]): string {
  const names: string[] = []
  const seen = new Set<string>()
  let id = folderId
  while (id && !seen.has(id)) {
    seen.add(id)
    const folder = folders.find((f) => f.id === id)
    if (!folder) break
    names.unshift(folder.name)
    id = folder.parentId
  }
  return names.join(' / ')
}

export function hostMatches(host: Host, query: string, tags: Tag[]): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  if (host.name.toLowerCase().includes(q) || host.address.toLowerCase().includes(q)) return true
  return (host.tagIds ?? []).some((id) =>
    tags
      .find((t) => t.id === id)
      ?.name.toLowerCase()
      .includes(q),
  )
}

export function typeGroup(t: ConnectionType): 'terminal' | 'files' | 'desktop' {
  if (t === 'ssh' || t === 'local') return 'terminal'
  if (t === 'rdp' || t === 'vnc') return 'desktop'
  return 'files'
}

export function viaLine(host: Host, hosts: Host[]): string | null {
  const main = host.connections[0]?.kind
  if (main?.type === 'ssh' && main.jumpHostId) {
    const jump = findConnection(hosts, main.jumpHostId)
    return jump ? `via ${jump.host.name}` : null
  }
  if (main?.type === 'rdp' && main.gateway) return `via ${main.gateway.host}`
  return null
}

export interface SshView {
  user: string
  host: string
  port: number
  appearance: ConnAppearance | null
  tunnels: Tunnel[]
}

export function sshView(
  hosts: Host[],
  logins: SavedLogin[],
  id: string | undefined,
): SshView | null {
  const found = id ? findConnection(hosts, id) : null
  const k = found?.connection.kind
  if (!found || k?.type !== 'ssh') return null
  return {
    user: loginUser(k.login, logins) ?? '',
    host: effectiveAddress(found.host, found.connection),
    port: k.port,
    appearance: k.appearance ?? null,
    tunnels: k.tunnels ?? [],
  }
}

export function s3View(
  hosts: Host[],
  id: string,
): { endpoint: string; port: number | null; useTls: boolean; pathStyle: boolean } | null {
  const found = findConnection(hosts, id)
  const k = found?.connection.kind
  if (!found || k?.type !== 's3') return null
  return {
    endpoint: effectiveAddress(found.host, found.connection),
    port: k.port,
    useTls: k.useTls,
    pathStyle: k.pathStyle,
  }
}

// Folder ids depth-first, siblings in their saved order.
export function folderOrder(folders: Folder[]): string[] {
  const known = new Set(folders.map((f) => f.id))
  const out: string[] = []
  const walk = (parentId: string | null) => {
    const children = folders
      .filter((f) =>
        parentId === null ? !f.parentId || !known.has(f.parentId) : f.parentId === parentId,
      )
      .sort((a, b) => a.order - b.order)
    for (const f of children) {
      out.push(f.id)
      walk(f.id)
    }
  }
  walk(null)
  return out
}
