import { open as pickFile } from '@tauri-apps/plugin-dialog'
import type { ConnectionDraft, HostData } from './drafts'
import { SelectField, TextField } from './fields'

const ANOTHER = 'another'
const NEW_KEY = 'new'

// "Saved login" appears only once saved logins exist; until then a connection has its own.
export function LoginControl({
  draft: d,
  onChange,
  errors,
  file,
  keyFiles,
}: {
  draft: ConnectionDraft
  onChange: (d: ConnectionDraft) => void
  errors: Record<string, string>
  file: HostData
  keyFiles: boolean
}) {
  const id = `c-${d.id}`
  const set = (patch: Partial<ConnectionDraft>) => onChange({ ...d, ...patch })
  const logins = keyFiles ? file.logins : file.logins.filter((l) => l.authMethod === 'password')
  const method = keyFiles ? d.authMethod : 'password'
  const keyValue = d.newKeyPath ? NEW_KEY : (d.keyIds[0] ?? '')
  const keyOptions: [string, string][] = [
    ...file.keys.map((k): [string, string] => [k.id, k.path]),
    ...(d.newKeyPath ? [[NEW_KEY, d.newKeyPath] as [string, string]] : []),
    [ANOTHER, 'Another file…'],
  ]

  return (
    <fieldset className="col-span-full flex flex-col gap-3">
      <legend className="mb-2 font-medium text-sm">Login</legend>
      <p className="text-muted-foreground text-xs">
        Manage keys and saved logins in Keys &amp; passwords.
      </p>
      {logins.length > 0 && (
        <div className="flex gap-4 text-sm">
          <label className="flex items-center gap-1.5">
            <input
              type="radio"
              checked={d.loginMode === 'saved'}
              onChange={() => set({ loginMode: 'saved', loginId: d.loginId || logins[0].id })}
            />
            Saved login
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="radio"
              checked={d.loginMode === 'own'}
              onChange={() => set({ loginMode: 'own' })}
            />
            Its own
          </label>
        </div>
      )}
      {d.loginMode === 'saved' && logins.length > 0 ? (
        <SelectField
          id={`${id}-login`}
          label="Saved login"
          value={d.loginId}
          onChange={(loginId) => set({ loginId })}
          options={logins.map((l) => [l.id, `${l.name} · ${l.username} · ${l.authMethod}`])}
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            id={`${id}-user`}
            label="User"
            value={d.user}
            onChange={(user) => set({ user })}
            error={errors.user}
          />
          {keyFiles && (
            <SelectField
              id={`${id}-auth`}
              label="Sign in with"
              value={d.authMethod}
              onChange={(v) => set({ authMethod: v as ConnectionDraft['authMethod'] })}
              options={[
                ['key', 'Key file'],
                ['password', 'Password'],
                ['agent', 'SSH agent'],
              ]}
            />
          )}
          {method === 'key' && (
            <SelectField
              id={`${id}-key`}
              label="Key file"
              value={keyValue}
              onChange={async (v) => {
                if (v !== ANOTHER) {
                  set(v === NEW_KEY ? {} : { keyIds: [v], newKeyPath: null })
                  return
                }
                const picked = await pickFile({ multiple: false, directory: false })
                if (typeof picked === 'string') set({ newKeyPath: picked })
              }}
              options={keyOptions}
            />
          )}
          {method === 'password' && (
            <TextField
              id={`${id}-password`}
              label="Password"
              type="password"
              value={d.password}
              onChange={(password) => set({ password })}
              placeholder={d.secretId ? 'Saved. Type to replace it.' : undefined}
            />
          )}
        </div>
      )}
    </fieldset>
  )
}
