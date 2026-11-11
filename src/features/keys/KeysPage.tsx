import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { useEffect, useState } from 'react'
import type { AuthMethod, Host, SavedLogin, SshKey } from '@/bindings'
import { commands } from '@/bindings'
import { Button } from '@/components/ui/button'
import { PageShell } from '@/components/ui/page-shell'
import { SelectField, TextField } from '@/features/connections/fields'
import { connectionLabel } from '@/lib/hosts'
import { unwrap } from '@/lib/ipc'
import { setSecret } from '@/lib/vault'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'

const fileName = (path: string) => path.split(/[\\/]/).pop() || path
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e))

const METHOD: Record<AuthMethod, string> = { password: 'Password', key: 'Key', agent: 'SSH agent' }

function useName() {
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  return (h: Host, c: Host['connections'][number]) =>
    `${h.name} · ${connectionLabel(h, c, logins, shells)}`
}

function Section({
  title,
  action,
  children,
}: {
  title: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold text-sm">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  )
}

const rowClass = 'flex items-center gap-3 rounded-lg border border-border p-3 text-sm'
const empty = 'text-muted-foreground text-sm'

function LoginEditor({
  login,
  keys,
  onDone,
}: {
  login: SavedLogin | null
  keys: SshKey[]
  onDone: () => void
}) {
  const saveLogin = useHostStore((s) => s.saveLogin)
  const deleteLogin = useHostStore((s) => s.deleteLogin)
  const [name, setName] = useState(login?.name ?? '')
  const [user, setUser] = useState(login?.username ?? '')
  const [method, setMethod] = useState<AuthMethod>(login?.authMethod ?? 'password')
  const [keyId, setKeyId] = useState(login?.keyIds?.[0] ?? keys[0]?.id ?? '')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const id = login?.id ?? 'new'

  const save = async () => {
    const secretId =
      method !== 'password'
        ? null
        : password
          ? await setSecret(password)
          : (login?.secretId ?? null)
    await saveLogin({
      id: login?.id ?? crypto.randomUUID(),
      name: name.trim(),
      username: user.trim(),
      authMethod: method,
      keyIds: method === 'key' && keyId ? [keyId] : [],
      secretId,
    })
    onDone()
  }

  return (
    <div className="grid gap-3 rounded-lg border border-border p-3 sm:grid-cols-2">
      <TextField id={`login-${id}-name`} label="Name" value={name} onChange={setName} />
      <TextField id={`login-${id}-user`} label="User" value={user} onChange={setUser} />
      <SelectField
        id={`login-${id}-method`}
        label="Sign in with"
        value={method}
        onChange={(v) => setMethod(v as AuthMethod)}
        options={[
          ['password', 'Password'],
          ['key', 'Key file'],
          ['agent', 'SSH agent'],
        ]}
      />
      {method === 'key' && (
        <SelectField
          id={`login-${id}-key`}
          label="Key file"
          value={keyId}
          onChange={setKeyId}
          options={keys.map((k) => [k.id, k.path])}
          error={keys.length === 0 ? 'Add a key file below first.' : undefined}
        />
      )}
      {method === 'password' && (
        <TextField
          id={`login-${id}-password`}
          label="Password"
          type="password"
          value={password}
          onChange={setPassword}
          placeholder={login?.secretId ? 'Saved. Type to replace it.' : undefined}
        />
      )}
      {error && (
        <p role="alert" className="col-span-full text-destructive text-sm">
          {error}
        </p>
      )}
      <div className="col-span-full flex justify-end gap-2">
        {login && (
          <Button
            type="button"
            variant="ghost"
            onClick={() =>
              deleteLogin(login.id).then(onDone, (e: unknown) => setError(errorText(e)))
            }
          >
            Delete
          </Button>
        )}
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          type="button"
          disabled={!name.trim() || !user.trim() || (method === 'key' && !keyId)}
          onClick={() => save().catch((e: unknown) => setError(errorText(e)))}
        >
          Save
        </Button>
      </div>
    </div>
  )
}

function KeyEditor({ sshKey, onDone }: { sshKey: SshKey; onDone: () => void }) {
  const saveKey = useHostStore((s) => s.saveKey)
  const [path, setPath] = useState(sshKey.path)
  const [passphrase, setPassphrase] = useState('')
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    const secretId = passphrase ? await setSecret(passphrase) : sshKey.secretId
    await saveKey({ ...sshKey, path: path.trim(), secretId })
    onDone()
  }
  return (
    <div className="grid gap-3 rounded-lg border border-border p-3 sm:grid-cols-2">
      <TextField id={`key-${sshKey.id}-path`} label="File" value={path} onChange={setPath} />
      <TextField
        id={`key-${sshKey.id}-passphrase`}
        label="Passphrase"
        type="password"
        value={passphrase}
        onChange={setPassphrase}
        placeholder={sshKey.secretId ? 'Saved. Type to replace it.' : 'Only if the key has one.'}
      />
      {error && (
        <p role="alert" className="col-span-full text-destructive text-sm">
          {error}
        </p>
      )}
      <div className="col-span-full flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          type="button"
          disabled={!path.trim()}
          onClick={() => save().catch((e: unknown) => setError(errorText(e)))}
        >
          Save
        </Button>
      </div>
    </div>
  )
}

export function KeysPage({ onClose }: { onClose: () => void }) {
  const hosts = useHostStore((s) => s.hosts)
  const logins = useHostStore((s) => s.logins)
  const keys = useHostStore((s) => s.keys)
  const saveKey = useHostStore((s) => s.saveKey)
  const deleteKey = useHostStore((s) => s.deleteKey)
  const openEditor = useHostsUiStore((s) => s.openEditor)
  const nameOf = useName()
  const [editingLogin, setEditingLogin] = useState<string | null>(null)
  const [editingKey, setEditingKey] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [usesPassword, setUsesPassword] = useState<boolean | null>(null)

  useEffect(() => {
    commands
      .vaultUsesPassword()
      .then((r) => setUsesPassword(unwrap(r)))
      .catch(console.error)
  }, [])

  const connections = hosts.flatMap((h) => h.connections.map((c) => ({ h, c })))
  const loginUsers = (id: string) =>
    connections.flatMap(({ h, c }) => {
      const k = c.kind
      const uses = (l: { kind: string; loginId?: string } | null | undefined) =>
        l?.kind === 'saved' && l.loginId === id
      const out: string[] = []
      if (k.type === 'ssh' || k.type === 'ftp' || k.type === 'rdp') {
        if (uses(k.login)) out.push(nameOf(h, c))
      }
      if (k.type === 'sftp' && uses(k.own?.login)) out.push(nameOf(h, c))
      if (k.type === 'rdp' && uses(k.gateway?.login)) out.push(`${nameOf(h, c)} gateway`)
      return out
    })
  const keyUsers = (id: string) => [
    ...logins.filter((l) => (l.keyIds ?? []).includes(id)).map((l) => `the login ${l.name}`),
    ...connections.flatMap(({ h, c }) => {
      const k = c.kind
      const login =
        k.type === 'ssh' || k.type === 'ftp' || k.type === 'rdp'
          ? k.login
          : k.type === 'sftp'
            ? k.own?.login
            : undefined
      return login?.kind === 'own' && (login.keyIds ?? []).includes(id) ? [nameOf(h, c)] : []
    }),
  ]
  const singles = connections.flatMap(({ h, c }) => {
    const k = c.kind
    const login =
      k.type === 'ssh' || k.type === 'ftp' || k.type === 'rdp'
        ? k.login
        : k.type === 'sftp'
          ? k.own?.login
          : undefined
    if (login?.kind === 'own' && login.secretId) return [{ h, c, what: 'Password' }]
    if (k.type === 'vnc' && k.secretId) return [{ h, c, what: 'Password' }]
    if (k.type === 's3' && k.secretId) return [{ h, c, what: 'Secret key' }]
    return []
  })

  const addKey = async () => {
    const picked = await pickFile({ multiple: false, directory: false })
    if (typeof picked !== 'string') return
    await saveKey({ id: crypto.randomUUID(), path: picked, secretId: null })
  }

  const kindOf = (l: SavedLogin) => {
    if (l.authMethod !== 'key') return METHOD[l.authMethod]
    const key = keys.find((k) => k.id === l.keyIds?.[0])
    return key ? `Key ${fileName(key.path)}` : 'Key'
  }

  return (
    <PageShell
      title="Keys & passwords"
      footer={
        <Button type="button" variant="ghost" onClick={onClose}>
          Back
        </Button>
      }
    >
      <div className="flex max-w-3xl flex-col gap-6">
        {usesPassword !== null && (
          <p className="text-muted-foreground text-sm">
            {usesPassword
              ? 'Passwords and passphrases are encrypted with your master password.'
              : 'Passwords and passphrases are encrypted with a key kept in the system keychain.'}
          </p>
        )}
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}

        <Section
          title="Saved logins"
          action={
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setEditingLogin('new')}
            >
              New login
            </Button>
          }
        >
          {editingLogin === 'new' && (
            <LoginEditor login={null} keys={keys} onDone={() => setEditingLogin(null)} />
          )}
          {logins.length === 0 && editingLogin !== 'new' && (
            <p className={empty}>
              No saved logins. A saved login keeps a user and a password or key that several
              connections share.
            </p>
          )}
          {logins.map((l) =>
            editingLogin === l.id ? (
              <LoginEditor key={l.id} login={l} keys={keys} onDone={() => setEditingLogin(null)} />
            ) : (
              <div key={l.id} className={rowClass}>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{l.name}</p>
                  <p className="truncate text-muted-foreground text-xs">
                    {l.username} · {kindOf(l)}
                  </p>
                  <p className="truncate text-muted-foreground text-xs">
                    {loginUsers(l.id).join(', ') || 'Not used'}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setEditingLogin(l.id)}
                >
                  Edit
                </Button>
              </div>
            ),
          )}
        </Section>

        <Section
          title="SSH keys"
          action={
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => addKey().catch((e: unknown) => setError(errorText(e)))}
            >
              Add a key file
            </Button>
          }
        >
          {keys.length === 0 && <p className={empty}>No key files yet.</p>}
          {keys.map((k) => {
            const users = keyUsers(k.id)
            return editingKey === k.id ? (
              <KeyEditor key={k.id} sshKey={k} onDone={() => setEditingKey(null)} />
            ) : (
              <div key={k.id} className={rowClass}>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{k.path}</p>
                  <p className="truncate text-muted-foreground text-xs">
                    Passphrase {k.secretId ? 'saved' : 'not saved'}
                  </p>
                  <p className="truncate text-muted-foreground text-xs">
                    {users.join(', ') || 'Not used'}
                  </p>
                </div>
                {users.length === 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => deleteKey(k.id).catch((e: unknown) => setError(errorText(e)))}
                  >
                    Remove
                  </Button>
                )}
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setEditingKey(k.id)}
                >
                  Edit
                </Button>
              </div>
            )
          })}
        </Section>

        <Section title="Passwords kept on a single connection">
          {singles.length === 0 && (
            <p className={empty}>No connection keeps a password of its own.</p>
          )}
          {singles.map(({ h, c, what }) => (
            <div key={c.id} className={rowClass}>
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium">{nameOf(h, c)}</p>
                <p className="truncate text-muted-foreground text-xs">{what}</p>
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => openEditor(h.id, c.id)}
              >
                Open connection
              </Button>
            </div>
          ))}
        </Section>
      </div>
    </PageShell>
  )
}
