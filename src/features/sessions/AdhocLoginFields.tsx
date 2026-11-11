import { open as pickFile } from '@tauri-apps/plugin-dialog'
import type { AuthMethod } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { AdhocParams } from '@/lib/sftp'
import { setSecret } from '@/lib/vault'

// What the form collects: the plaintext secret is exchanged for a vault id before it
// reaches the session store.
export interface AdhocForm {
  host: string
  port: number
  username: string
  authMethod: AuthMethod
  keyPath: string
  secret: string
}

export interface AdhocInitial {
  host?: string
  port?: number
  username?: string
}

export const adhocForm = (port: number, initial?: AdhocInitial): AdhocForm => ({
  host: initial?.host ?? '',
  port: initial?.port ?? port,
  username: initial?.username ?? '',
  authMethod: 'password',
  keyPath: '',
  secret: '',
})

export const adhocReady = (f: AdhocForm) =>
  !!f.host &&
  !!f.username &&
  Number.isInteger(f.port) &&
  f.port >= 1 &&
  f.port <= 65535 &&
  (f.authMethod !== 'key' || !!f.keyPath)

export async function toAdhocParams({
  secret,
  keyPath,
  authMethod,
  host,
  port,
  username,
}: AdhocForm): Promise<AdhocParams> {
  const secretId = authMethod !== 'agent' && secret ? await setSecret(secret) : null
  return {
    host,
    port,
    username,
    authMethod,
    // Quick connect takes one key; its passphrase is the same secret.
    keys: authMethod === 'key' && keyPath ? [{ path: keyPath, secretId }] : [],
    secretId: authMethod === 'key' ? null : secretId,
  }
}

export function AdhocLoginFields({
  idPrefix,
  value: f,
  onChange,
}: {
  idPrefix: string
  value: AdhocForm
  onChange: (f: AdhocForm) => void
}) {
  const set = (patch: Partial<AdhocForm>) => onChange({ ...f, ...patch })
  return (
    <div className="space-y-2">
      <div className="space-y-1">
        <Label htmlFor={`${idPrefix}-host`}>Host</Label>
        <Input
          id={`${idPrefix}-host`}
          autoFocus={!f.host}
          value={f.host}
          onChange={(e) => set({ host: e.target.value })}
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`${idPrefix}-port`}>Port</Label>
        <Input
          id={`${idPrefix}-port`}
          type="number"
          value={f.port}
          onChange={(e) => set({ port: Number(e.target.value) || 22 })}
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`${idPrefix}-username`}>Username</Label>
        <Input
          id={`${idPrefix}-username`}
          autoFocus={!!f.host && !f.username}
          value={f.username}
          onChange={(e) => set({ username: e.target.value })}
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`${idPrefix}-auth`}>Auth method</Label>
        <Select value={f.authMethod} onValueChange={(v) => set({ authMethod: v as AuthMethod })}>
          <SelectTrigger id={`${idPrefix}-auth`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="password">Password</SelectItem>
            <SelectItem value="key">Private key</SelectItem>
            <SelectItem value="agent">SSH agent</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {f.authMethod === 'key' && (
        <div className="space-y-1">
          <Label htmlFor={`${idPrefix}-key-path`}>Key path</Label>
          <div className="flex gap-2">
            <Input
              id={`${idPrefix}-key-path`}
              value={f.keyPath}
              onChange={(e) => set({ keyPath: e.target.value })}
            />
            <Button
              type="button"
              variant="secondary"
              onClick={async () => {
                const picked = await pickFile({ multiple: false, directory: false })
                if (typeof picked === 'string') set({ keyPath: picked })
              }}
            >
              Browse
            </Button>
          </div>
        </div>
      )}
      {f.authMethod !== 'agent' && (
        <div className="space-y-1">
          <Label htmlFor={`${idPrefix}-secret`}>
            {f.authMethod === 'key' ? 'Passphrase' : 'Password'}
          </Label>
          <Input
            id={`${idPrefix}-secret`}
            type="password"
            autoFocus={!!f.host && !!f.username}
            value={f.secret}
            onChange={(e) => set({ secret: e.target.value })}
          />
        </div>
      )}
    </div>
  )
}
