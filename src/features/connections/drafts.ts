import {
  type AuthMethod,
  type Connection,
  commands,
  type Host,
  type Login,
  type SavedLogin,
  type SshKey,
  type Tunnel,
  type TunnelKind,
} from '@/bindings'
import { type ConnectionType, connectionLabel } from '@/lib/hosts'
import { unwrap } from '@/lib/ipc'
import { setSecret } from '@/lib/vault'

// What the form reads from the host store.
export interface HostData {
  hosts: Host[]
  logins: SavedLogin[]
  keys: SshKey[]
}

export const DEFAULT_PORTS: Record<Exclude<ConnectionType, 'local'>, string> = {
  ssh: '22',
  sftp: '22',
  ftp: '21',
  s3: '',
  rdp: '3389',
  vnc: '5900',
}

export interface TunnelDraft {
  id: string
  kind: TunnelKind
  bindHost: string
  bindPort: string
  targetHost: string
  targetPort: string
  autoStart: boolean
}

// Form state for one connection: numbers as typed text, and secrets as typed text that is
// stored only when the connection is saved.
export interface ConnectionDraft {
  id: string
  type: ConnectionType
  original: Connection | null
  label: string
  address: string
  port: string
  loginMode: 'own' | 'saved'
  loginId: string
  user: string
  authMethod: AuthMethod
  keyIds: string[]
  newKeyPath: string | null
  password: string
  secretId: string | null
  sameAsSsh: boolean
  startFolder: string
  secure: boolean
  allowInvalidCert: boolean
  ignoreHostname: boolean
  bucket: string
  region: string
  accessKeyId: string
  useTls: boolean
  pathStyle: boolean
  domain: string
  keyboardLayout: string
  audio: boolean
  share: boolean
  sharePath: string
  shareName: string
  gateway: boolean
  gatewayHost: string
  gatewayPort: string
  gatewaySame: boolean
  gatewayUser: string
  gatewayPassword: string
  gatewaySecretId: string | null
  program: string
  jumpHostId: string
  fontFamily: string
  fontSize: string
  tunnels: TunnelDraft[]
}

function loginFields(login: Login | null | undefined, file: HostData) {
  if (!login)
    return {
      loginMode: 'own' as const,
      loginId: '',
      user: '',
      authMethod: 'password' as AuthMethod,
      keyIds: [],
      secretId: null,
    }
  if (login.kind === 'saved') {
    const saved = file.logins.find((l) => l.id === login.loginId)
    return {
      loginMode: 'saved' as const,
      loginId: login.loginId,
      user: saved?.username ?? '',
      authMethod: saved?.authMethod ?? 'password',
      keyIds: [],
      secretId: null,
    }
  }
  return {
    loginMode: 'own' as const,
    loginId: '',
    user: login.username,
    authMethod: login.authMethod,
    keyIds: login.keyIds ?? [],
    secretId: login.secretId,
  }
}

const text = (v: string | number | null | undefined) =>
  v === null || v === undefined ? '' : String(v)

export function draftFrom(
  type: ConnectionType,
  c: Connection | null,
  host: Host | null,
  file: HostData,
): ConnectionDraft {
  const d: ConnectionDraft = {
    id: c?.id ?? crypto.randomUUID(),
    type,
    original: c,
    label: c?.label ?? '',
    address: c?.address ?? '',
    port: type === 'local' ? '' : DEFAULT_PORTS[type],
    ...loginFields(null, file),
    newKeyPath: null,
    password: '',
    sameAsSsh: !c && type === 'sftp' && !!host?.connections.some((x) => x.kind.type === 'ssh'),
    startFolder: '',
    secure: false,
    allowInvalidCert: false,
    ignoreHostname: false,
    bucket: '',
    region: '',
    accessKeyId: '',
    useTls: true,
    pathStyle: false,
    domain: '',
    keyboardLayout: '',
    audio: true,
    share: false,
    sharePath: '',
    shareName: '',
    gateway: false,
    gatewayHost: '',
    gatewayPort: '443',
    gatewaySame: true,
    gatewayUser: '',
    gatewayPassword: '',
    gatewaySecretId: null,
    program: '',
    jumpHostId: '',
    fontFamily: '',
    fontSize: '',
    tunnels: [],
  }
  const k = c?.kind
  if (!k) return d
  switch (k.type) {
    case 'ssh':
      return {
        ...d,
        ...loginFields(k.login, file),
        port: text(k.port),
        jumpHostId: k.jumpHostId ?? '',
        fontFamily: k.appearance?.fontFamily ?? '',
        fontSize: text(k.appearance?.fontSize),
        tunnels: (k.tunnels ?? []).map(tunnelDraft),
      }
    case 'sftp':
      return {
        ...d,
        ...loginFields(k.own?.login, file),
        sameAsSsh: !k.own,
        port: k.own ? text(k.own.port) : d.port,
        startFolder: k.startFolder ?? '',
      }
    case 'ftp':
      return {
        ...d,
        ...loginFields(k.login, file),
        port: text(k.port),
        secure: k.secure,
        allowInvalidCert: k.allowInvalidCert,
        ignoreHostname: k.ignoreHostname,
      }
    case 's3':
      return {
        ...d,
        port: text(k.port),
        bucket: k.bucket ?? '',
        region: k.region,
        accessKeyId: k.accessKeyId,
        secretId: k.secretId,
        useTls: k.useTls,
        pathStyle: k.pathStyle,
      }
    case 'rdp': {
      const g = k.gateway ?? null
      const gl = g?.login ? loginFields(g.login, file) : null
      return {
        ...d,
        ...loginFields(k.login, file),
        port: text(k.port),
        domain: k.domain ?? '',
        keyboardLayout: text(k.keyboardLayout),
        audio: k.audio ?? true,
        share: !!k.sharedFolder,
        sharePath: k.sharedFolder?.path ?? '',
        shareName: k.sharedFolder?.name ?? '',
        gateway: !!g,
        gatewayHost: g?.host ?? '',
        gatewayPort: g ? text(g.port) : '443',
        gatewaySame: !g?.login,
        gatewayUser: gl?.user ?? '',
        gatewaySecretId: gl?.secretId ?? null,
      }
    }
    case 'vnc':
      return { ...d, port: text(k.port), user: k.username ?? '', secretId: k.secretId }
    case 'local':
      return { ...d, program: k.program, startFolder: k.startFolder ?? '' }
  }
}

const PORT_ERROR = 'Use a number from 1 to 65535.'
const REQUIRED = 'Fill this in.'

function badPort(text: string): boolean {
  if (!/^\d+$/.test(text)) return true
  const n = Number(text)
  return n < 1 || n > 65535
}

const ownLogin = (d: ConnectionDraft) =>
  (d.type === 'ssh' ||
    d.type === 'ftp' ||
    d.type === 'rdp' ||
    (d.type === 'sftp' && !d.sameAsSsh)) &&
  d.loginMode === 'own'

export function validateDraft(d: ConnectionDraft): Record<string, string> {
  const errors: Record<string, string> = {}
  if (d.type === 'local') {
    if (!d.program) errors.program = REQUIRED
    return errors
  }
  const portNeeded = !(d.type === 's3' && d.port === '') && !(d.type === 'sftp' && d.sameAsSsh)
  if (portNeeded && badPort(d.port)) errors.port = PORT_ERROR
  if (ownLogin(d) && !d.user.trim()) errors.user = REQUIRED
  if (d.type === 's3' && !d.region.trim()) errors.region = REQUIRED
  if (d.type === 'rdp' && d.share && !d.sharePath) errors.sharePath = REQUIRED
  if (d.type === 'rdp' && d.gateway) {
    if (!d.gatewayHost.trim()) errors.gatewayHost = REQUIRED
    if (badPort(d.gatewayPort)) errors.gatewayPort = PORT_ERROR
    if (!d.gatewaySame && !d.gatewayUser.trim()) errors.gatewayUser = REQUIRED
  }
  if (d.type === 'ssh') Object.assign(errors, tunnelErrors(d.tunnels))
  return errors
}

export function tunnelErrors(tunnels: TunnelDraft[]): Record<string, string> {
  const errors: Record<string, string> = {}
  for (const t of tunnels) {
    if (badPort(t.bindPort)) errors[`tunnel-${t.id}-bindPort`] = PORT_ERROR
    if (t.kind !== 'dynamic' && badPort(t.targetPort))
      errors[`tunnel-${t.id}-targetPort`] = PORT_ERROR
  }
  return errors
}

export function tunnelDraft(t: Tunnel): TunnelDraft {
  return {
    ...t,
    bindPort: text(t.bindPort),
    targetHost: t.targetHost ?? '',
    targetPort: text(t.targetPort),
  }
}

export function tunnelFrom(t: TunnelDraft): Tunnel {
  return {
    id: t.id,
    kind: t.kind,
    bindHost: t.bindHost.trim(),
    bindPort: Number(t.bindPort),
    targetHost: t.kind === 'dynamic' ? null : t.targetHost.trim(),
    targetPort: t.kind === 'dynamic' ? null : Number(t.targetPort),
    autoStart: t.autoStart,
  }
}

async function keyIdFor(path: string, file: HostData): Promise<string> {
  const known = file.keys.find((k) => k.path === path)
  if (known) return known.id
  const key: SshKey = { id: crypto.randomUUID(), path, secretId: null }
  unwrap(await commands.upsertKey(key))
  return key.id
}

async function loginOf(d: ConnectionDraft, file: HostData): Promise<Login> {
  if (d.loginMode === 'saved') return { kind: 'saved', loginId: d.loginId }
  const keyIds = d.newKeyPath ? [await keyIdFor(d.newKeyPath, file)] : d.keyIds
  return {
    kind: 'own',
    username: d.user.trim(),
    authMethod: d.authMethod,
    keyIds: d.authMethod === 'key' ? keyIds : [],
    secretId: d.password ? await setSecret(d.password) : d.secretId,
  }
}

const orNull = (s: string) => (s.trim() ? s.trim() : null)

// Typed secrets go to the vault here; the store drops the ones they replace when it saves.
export async function toConnection(d: ConnectionDraft, file: HostData): Promise<Connection> {
  const base = { id: d.id, label: orNull(d.label), address: orNull(d.address) }
  const port = Number(d.port)
  switch (d.type) {
    case 'ssh': {
      const old = d.original?.kind.type === 'ssh' ? d.original.kind.appearance : null
      const fontSize = d.fontSize ? Number(d.fontSize) : null
      const appearance =
        old?.colors || d.fontFamily || fontSize
          ? { colors: old?.colors ?? null, fontFamily: orNull(d.fontFamily), fontSize }
          : null
      return {
        ...base,
        kind: {
          type: 'ssh',
          port,
          login: await loginOf(d, file),
          jumpHostId: d.jumpHostId || null,
          appearance,
          tunnels: d.tunnels.map(tunnelFrom),
        },
      }
    }
    case 'sftp':
      return {
        ...base,
        kind: {
          type: 'sftp',
          own: d.sameAsSsh ? null : { port, login: await loginOf(d, file) },
          startFolder: orNull(d.startFolder),
        },
      }
    case 'ftp':
      return {
        ...base,
        kind: {
          type: 'ftp',
          port,
          login: await loginOf({ ...d, authMethod: 'password' }, file),
          secure: d.secure,
          allowInvalidCert: d.allowInvalidCert,
          ignoreHostname: d.ignoreHostname,
        },
      }
    case 's3':
      return {
        ...base,
        kind: {
          type: 's3',
          port: d.port ? port : null,
          region: d.region.trim(),
          useTls: d.useTls,
          pathStyle: d.pathStyle,
          accessKeyId: d.accessKeyId.trim(),
          secretId: d.password ? await setSecret(d.password) : d.secretId,
          bucket: orNull(d.bucket),
        },
      }
    case 'rdp': {
      let gateway = null
      if (d.gateway) {
        const login: Login | null = d.gatewaySame
          ? null
          : {
              kind: 'own',
              username: d.gatewayUser.trim(),
              authMethod: 'password',
              keyIds: [],
              secretId: d.gatewayPassword ? await setSecret(d.gatewayPassword) : d.gatewaySecretId,
            }
        gateway = { host: d.gatewayHost.trim(), port: Number(d.gatewayPort), login }
      }
      return {
        ...base,
        kind: {
          type: 'rdp',
          port,
          login: await loginOf({ ...d, authMethod: 'password' }, file),
          domain: orNull(d.domain),
          keyboardLayout: d.keyboardLayout ? Number(d.keyboardLayout) : null,
          audio: d.audio,
          sharedFolder: d.share ? { path: d.sharePath, name: d.shareName || 'share' } : null,
          gateway,
        },
      }
    }
    case 'vnc':
      return {
        ...base,
        kind: {
          type: 'vnc',
          port,
          username: orNull(d.user),
          secretId: d.password ? await setSecret(d.password) : d.secretId,
        },
      }
    case 'local':
      return {
        ...base,
        address: null,
        kind: { type: 'local', program: d.program, startFolder: orNull(d.startFolder) },
      }
  }
}

// Every SSH connection this one could jump through: not itself, and none whose own chain
// already comes back to it.
export function jumpChoices(
  file: HostData,
  connectionId: string | null,
): { id: string; label: string }[] {
  const jumpOf = new Map<string, string | null>()
  for (const h of file.hosts) {
    for (const c of h.connections) {
      if (c.kind.type === 'ssh') jumpOf.set(c.id, c.kind.jumpHostId)
    }
  }
  const loops = (candidate: string) => {
    const seen = new Set<string>()
    let id: string | null | undefined = candidate
    while (id) {
      if (id === connectionId || seen.has(id)) return true
      seen.add(id)
      id = jumpOf.get(id)
    }
    return false
  }
  return file.hosts.flatMap((h) =>
    h.connections
      .filter((c) => c.kind.type === 'ssh' && !loops(c.id))
      .map((c) => ({ id: c.id, label: `${h.name} · ${connectionLabel(h, c, file.logins)}` })),
  )
}
