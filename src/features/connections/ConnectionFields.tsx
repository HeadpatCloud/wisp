import { open as pickFolder } from '@tauri-apps/plugin-dialog'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Host, ShellInfo } from '@/bindings'
import { Button } from '@/components/ui/button'
import { loginUser, typeName } from '@/lib/hosts'
import { LAYOUTS } from '@/lib/rdpLayouts'
import { type ConnectionDraft, type HostData, jumpChoices } from './drafts'
import { Check, SelectField, TextField } from './fields'
import { LoginControl } from './LoginControl'
import { TunnelRows } from './TunnelRows'

const MORE: Partial<Record<ConnectionDraft['type'], string>> = {
  ssh: 'jump host, different address, font, tunnels',
  sftp: 'different address',
  ftp: 'different address',
  s3: 'different address',
  rdp: 'different address, gateway login',
  vnc: 'different address',
}

const NONE = 'none'
const SAME = 'same'

// RDP share names are up to 7 letters, digits, - or _ (rdpdr).
const shareNameOf = (path: string) =>
  (path.split(/[\\/]/).filter(Boolean).at(-1) ?? '').replace(/[^A-Za-z0-9_-]/g, '').slice(0, 7) ||
  'share'

export function ConnectionFields({
  draft: d,
  onChange,
  errors,
  host,
  file,
  shells,
}: {
  draft: ConnectionDraft
  onChange: (d: ConnectionDraft) => void
  errors: Record<string, string>
  host: Host | null
  file: HostData
  shells: ShellInfo[]
}) {
  const [more, setMore] = useState(false)
  const id = `c-${d.id}`
  const set = (patch: Partial<ConnectionDraft>) => onChange({ ...d, ...patch })
  // An error on a field under More options must be seen, or Save looks like it does nothing.
  const hiddenError = Object.keys(errors).some(
    (k) => k.startsWith('tunnel-') || k === 'gatewayUser',
  )
  useEffect(() => {
    if (hiddenError) setMore(true)
  }, [hiddenError])
  const firstShell = shells[0]?.program
  // biome-ignore lint/correctness/useExhaustiveDependencies: fills an empty shell once, without overwriting a choice
  useEffect(() => {
    if (d.type === 'local' && !d.program && firstShell) set({ program: firstShell })
  }, [d.type, d.program, firstShell])
  const login = (keyFiles: boolean) => (
    <LoginControl draft={d} onChange={onChange} errors={errors} file={file} keyFiles={keyFiles} />
  )
  const port = (
    <TextField
      id={`${id}-port`}
      label="Port"
      value={d.port}
      onChange={(port) => set({ port })}
      error={errors.port}
      placeholder={d.type === 's3' ? 'Default' : undefined}
    />
  )
  const label = (
    <TextField
      id={`${id}-label`}
      label="Label"
      value={d.label}
      onChange={(label) => set({ label })}
      hint={d.label.trim() ? `Shown as "${typeName(d.type)} · ${d.label.trim()}"` : undefined}
    />
  )
  const firstSsh = host?.connections.find((c) => c.kind.type === 'ssh')
  const sshUser =
    firstSsh?.kind.type === 'ssh' ? (loginUser(firstSsh.kind.login, file.logins) ?? '') : ''

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {d.type === 'ssh' && (
        <>
          {port}
          {label}
          {login(true)}
        </>
      )}
      {d.type === 'sftp' && (
        <>
          {/* Shown while on even without SSH, so a login can still be given to Files. */}
          {(firstSsh || d.sameAsSsh) && (
            <div className="col-span-full">
              <Check
                label={`Same login as SSH${sshUser ? ` · ${sshUser}` : ''}`}
                hint="Uses its port, user and key. Turn off to set them here."
                checked={d.sameAsSsh}
                onChange={(sameAsSsh) => set({ sameAsSsh })}
              />
            </div>
          )}
          {!d.sameAsSsh && (
            <>
              {port}
              {login(true)}
            </>
          )}
          <TextField
            id={`${id}-start`}
            label="Start folder"
            value={d.startFolder}
            onChange={(startFolder) => set({ startFolder })}
          />
          {label}
        </>
      )}
      {d.type === 'ftp' && (
        <>
          {login(false)}
          {port}
          {label}
          <div className="col-span-full flex flex-col gap-2">
            <Check
              label="Encrypt with TLS (FTPS)"
              checked={d.secure}
              onChange={(secure) => set({ secure })}
            />
            {d.secure && (
              <>
                <Check
                  label="Accept a certificate that doesn't check out"
                  checked={d.allowInvalidCert}
                  onChange={(allowInvalidCert) => set({ allowInvalidCert })}
                />
                <Check
                  label="Ignore a certificate name mismatch"
                  checked={d.ignoreHostname}
                  onChange={(ignoreHostname) => set({ ignoreHostname })}
                />
              </>
            )}
          </div>
        </>
      )}
      {d.type === 's3' && (
        <>
          <TextField
            id={`${id}-bucket`}
            label="Bucket"
            value={d.bucket}
            onChange={(bucket) => set({ bucket })}
            hint="Empty lists every bucket"
          />
          <TextField
            id={`${id}-region`}
            label="Region"
            value={d.region}
            onChange={(region) => set({ region })}
            error={errors.region}
          />
          <TextField
            id={`${id}-access`}
            label="Access key ID"
            value={d.accessKeyId}
            onChange={(accessKeyId) => set({ accessKeyId })}
          />
          <TextField
            id={`${id}-secret`}
            label="Secret key"
            type="password"
            value={d.password}
            onChange={(password) => set({ password })}
            placeholder={d.secretId ? 'Saved. Type to replace it.' : undefined}
          />
          {port}
          {label}
          <div className="col-span-full flex flex-col gap-2">
            <Check label="Use TLS" checked={d.useTls} onChange={(useTls) => set({ useTls })} />
            <Check
              label="Path-style addresses (MinIO and similar)"
              checked={d.pathStyle}
              onChange={(pathStyle) => set({ pathStyle })}
            />
          </div>
        </>
      )}
      {d.type === 'rdp' && (
        <>
          {login(false)}
          <TextField
            id={`${id}-domain`}
            label="Domain"
            value={d.domain}
            onChange={(domain) => set({ domain })}
            placeholder="Optional"
          />
          {port}
          <SelectField
            id={`${id}-layout`}
            label="Keyboard layout"
            value={d.keyboardLayout || SAME}
            onChange={(v) => set({ keyboardLayout: v === SAME ? '' : v })}
            options={[
              [SAME, 'Same as this computer'],
              ...LAYOUTS.map(([code, name]): [string, string] => [String(code), name]),
            ]}
          />
          {label}
          <div className="col-span-full flex flex-col gap-2">
            <Check
              label="Play sound from the remote computer"
              checked={d.audio}
              onChange={(audio) => set({ audio })}
            />
            <div className="flex flex-wrap items-center gap-2">
              <Check
                label={d.share && d.sharePath ? `Share a folder: ${d.sharePath}` : 'Share a folder'}
                checked={d.share}
                onChange={(share) => set({ share })}
              />
              {d.share && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={async () => {
                    const picked = await pickFolder({ directory: true })
                    if (typeof picked === 'string') {
                      set({ sharePath: picked, shareName: shareNameOf(picked) })
                    }
                  }}
                >
                  {d.sharePath ? 'Change' : 'Choose a folder…'}
                </Button>
              )}
            </div>
            {errors.sharePath && <p className="text-destructive text-xs">{errors.sharePath}</p>}
            <Check
              label="Connect through an RD Gateway"
              checked={d.gateway}
              onChange={(gateway) => set({ gateway })}
            />
          </div>
          {d.gateway && (
            <>
              <TextField
                id={`${id}-gw-host`}
                label="Gateway"
                value={d.gatewayHost}
                onChange={(gatewayHost) => set({ gatewayHost })}
                error={errors.gatewayHost}
              />
              <TextField
                id={`${id}-gw-port`}
                label="Gateway port"
                value={d.gatewayPort}
                onChange={(gatewayPort) => set({ gatewayPort })}
                error={errors.gatewayPort}
              />
            </>
          )}
        </>
      )}
      {d.type === 'vnc' && (
        <>
          {port}
          <TextField
            id={`${id}-user`}
            label="User"
            value={d.user}
            onChange={(user) => set({ user })}
            placeholder="Only if the server asks"
          />
          <TextField
            id={`${id}-password`}
            label="Password"
            type="password"
            value={d.password}
            onChange={(password) => set({ password })}
            placeholder={d.secretId ? 'Saved. Type to replace it.' : undefined}
          />
          {label}
        </>
      )}
      {d.type === 'local' && (
        <>
          <SelectField
            id={`${id}-shell`}
            label="Shell"
            value={d.program}
            onChange={(program) => set({ program })}
            error={errors.program}
            options={[
              ...shells.map((s): [string, string] => [s.program, s.name]),
              ...(d.program && !shells.some((s) => s.program === d.program)
                ? [[d.program, d.program] as [string, string]]
                : []),
            ]}
          />
          <TextField
            id={`${id}-start`}
            label="Start folder"
            value={d.startFolder}
            onChange={(startFolder) => set({ startFolder })}
          />
          {label}
          <p className="col-span-full text-muted-foreground text-sm">
            Saved under This computer, at the top of Hosts.
          </p>
        </>
      )}

      {MORE[d.type] && (
        <div className="col-span-full flex flex-col gap-3">
          <button
            type="button"
            aria-expanded={more}
            onClick={() => setMore(!more)}
            className="flex items-center gap-1 self-start text-sm hover:underline"
          >
            {more ? (
              <ChevronDown className="size-4" aria-hidden />
            ) : (
              <ChevronRight className="size-4" aria-hidden />
            )}
            More options
            <span className="text-muted-foreground">{MORE[d.type]}</span>
          </button>
          {more && (
            <div className="grid gap-3 sm:grid-cols-2">
              <TextField
                id={`${id}-address`}
                label="Different address"
                value={d.address}
                onChange={(address) => set({ address })}
                placeholder={host?.address ? `Uses ${host.address}` : undefined}
              />
              {d.type === 'ssh' && (
                <>
                  <SelectField
                    id={`${id}-jump`}
                    label="Jump host"
                    value={d.jumpHostId || NONE}
                    onChange={(v) => set({ jumpHostId: v === NONE ? '' : v })}
                    options={[
                      [NONE, 'None'],
                      ...jumpChoices(file, d.id).map((j): [string, string] => [j.id, j.label]),
                    ]}
                  />
                  <TextField
                    id={`${id}-font`}
                    label="Font"
                    value={d.fontFamily}
                    onChange={(fontFamily) => set({ fontFamily })}
                    placeholder="From settings"
                  />
                  <TextField
                    id={`${id}-font-size`}
                    label="Font size"
                    value={d.fontSize}
                    onChange={(fontSize) => set({ fontSize })}
                    placeholder="From settings"
                  />
                  <TunnelRows
                    tunnels={d.tunnels}
                    onChange={(tunnels) => set({ tunnels })}
                    errors={errors}
                  />
                </>
              )}
              {d.type === 'rdp' && d.gateway && (
                <div className="col-span-full flex flex-col gap-3">
                  <Check
                    label="Same login as the remote desktop"
                    checked={d.gatewaySame}
                    onChange={(gatewaySame) => set({ gatewaySame })}
                  />
                  {!d.gatewaySame && (
                    <div className="grid gap-3 sm:grid-cols-2">
                      <TextField
                        id={`${id}-gw-user`}
                        label="Gateway user"
                        value={d.gatewayUser}
                        onChange={(gatewayUser) => set({ gatewayUser })}
                        error={errors.gatewayUser}
                      />
                      <TextField
                        id={`${id}-gw-password`}
                        label="Gateway password"
                        type="password"
                        value={d.gatewayPassword}
                        onChange={(gatewayPassword) => set({ gatewayPassword })}
                        placeholder={d.gatewaySecretId ? 'Saved. Type to replace it.' : undefined}
                      />
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
