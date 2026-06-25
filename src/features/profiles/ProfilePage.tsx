import { zodResolver } from '@hookform/resolvers/zod'
import { open } from '@tauri-apps/plugin-dialog'
import type { ReactNode } from 'react'
import { useEffect, useMemo, useState } from 'react'
import { Controller, useFieldArray, useForm } from 'react-hook-form'
import { z } from 'zod'
import { commands, type IconRef, type Profile } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { unwrap } from '@/lib/ipc'
import { nextProfileOrder, wouldCycle } from '@/lib/profiles'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { IconPicker } from './IconPicker'

const schema = z.object({
  name: z.string().min(1, 'Name is required'),
  host: z.string().min(1, 'Host is required'),
  port: z.number().int().min(1).max(65535),
  username: z.string().min(1, 'Username is required'),
  authMethod: z.enum(['password', 'key', 'agent']),
  keyPath: z.string(),
  password: z.string(),
  groupId: z.string(),
  jumpHostId: z.string(),
  appFontFamily: z.string(),
  appFontSize: z.number().int().min(0).max(99),
  tunnels: z.array(
    z.object({
      id: z.string(),
      kind: z.enum(['local', 'remote', 'dynamic']),
      bindHost: z.string().min(1),
      bindPort: z.number().int().min(1).max(65535),
      targetHost: z.string(),
      targetPort: z.number().int().min(0).max(65535),
      autoStart: z.boolean(),
    }),
  ),
})

type FormValues = z.infer<typeof schema>

export function ProfilePage({ profileId, tabId }: { profileId: string | null; tabId: string }) {
  const saveProfile = useProfileStore((s) => s.saveProfile)
  const profiles = useProfileStore((s) => s.profiles)
  const groups = useProfileStore((s) => s.groups)
  const removeTab = useSessionStore((s) => s.removeTab)
  const profile = profileId ? (profiles.find((p) => p.id === profileId) ?? null) : null
  const [iconRef, setIconRef] = useState<IconRef>(
    profile?.icon ?? { kind: 'builtin', name: 'server' },
  )

  const formValues = useMemo<FormValues>(
    () => ({
      name: profile?.name ?? '',
      host: profile?.host ?? '',
      port: profile?.port ?? 22,
      username: profile?.username ?? '',
      authMethod: profile?.authMethod ?? 'password',
      keyPath: profile?.keyPath ?? '',
      password: '',
      groupId: profile?.groupId ?? '',
      jumpHostId: profile?.jumpHostId ?? '',
      appFontFamily: profile?.appearance?.fontFamily ?? '',
      appFontSize: profile?.appearance?.fontSize ?? 0,
      tunnels:
        profile?.tunnels?.map((t) => ({
          ...t,
          targetHost: t.targetHost ?? '',
          targetPort: t.targetPort ?? 0,
        })) ?? [],
    }),
    [profile],
  )

  const {
    register,
    handleSubmit,
    watch,
    setValue,
    control,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    resolver: zodResolver(schema),
    values: formValues,
  })

  const { fields, append, remove } = useFieldArray({ control, name: 'tunnels' })

  useEffect(() => {
    setIconRef(profile?.icon ?? { kind: 'builtin', name: 'server' })
  }, [profile])

  const authMethod = watch('authMethod')

  async function onSubmit(values: FormValues) {
    let secretId = profile?.secretId ?? null
    if (values.password) {
      const newId = unwrap(await commands.setSecret(values.password))
      if (profile?.secretId) await commands.deleteSecret(profile.secretId)
      secretId = newId
    }
    const appearance =
      values.appFontFamily || values.appFontSize
        ? {
            theme: null,
            fontFamily: values.appFontFamily || null,
            fontSize: values.appFontSize || null,
          }
        : null
    const next: Profile = {
      id: profile?.id ?? crypto.randomUUID(),
      name: values.name,
      appearance,
      groupId: values.groupId || null,
      host: values.host,
      port: values.port,
      username: values.username,
      authMethod: values.authMethod,
      keyPath: values.authMethod === 'key' ? values.keyPath || null : null,
      secretId,
      icon: iconRef,
      order: profile?.order ?? nextProfileOrder(profiles, values.groupId || null),
      jumpHostId: values.jumpHostId || null,
      tunnels: values.tunnels.map((t) => ({
        id: t.id || crypto.randomUUID(),
        kind: t.kind,
        bindHost: t.bindHost,
        bindPort: t.bindPort,
        targetHost: t.kind === 'dynamic' ? null : t.targetHost,
        targetPort: t.kind === 'dynamic' ? null : t.targetPort,
        autoStart: t.autoStart,
      })),
    }
    await saveProfile(next)
    removeTab(tabId)
  }

  return (
    <PageShell
      title={profile ? 'Edit profile' : 'New profile'}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
            Cancel
          </Button>
          <Button type="submit" form="profile-form" disabled={isSubmitting}>
            Save
          </Button>
        </>
      }
    >
      <form id="profile-form" className="space-y-3" onSubmit={handleSubmit(onSubmit)} noValidate>
        <Field htmlFor="name" label="Name" error={errors.name?.message}>
          <Input id="name" {...register('name')} />
        </Field>
        <Field htmlFor="host" label="Host" error={errors.host?.message}>
          <Input id="host" {...register('host')} />
        </Field>
        <Field htmlFor="port" label="Port" error={errors.port?.message}>
          <Input id="port" type="number" {...register('port', { valueAsNumber: true })} />
        </Field>
        <Field htmlFor="username" label="Username" error={errors.username?.message}>
          <Input id="username" {...register('username')} />
        </Field>
        <div className="space-y-1">
          <Label htmlFor="group">Group</Label>
          <Controller
            control={control}
            name="groupId"
            render={({ field }) => (
              <Select
                value={field.value || 'none'}
                onValueChange={(v) => field.onChange(v === 'none' ? '' : v)}
              >
                <SelectTrigger id="group">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None</SelectItem>
                  {groups.map((g) => (
                    <SelectItem key={g.id} value={g.id}>
                      {g.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="authMethod">Auth method</Label>
          <Controller
            control={control}
            name="authMethod"
            render={({ field }) => (
              <Select value={field.value} onValueChange={field.onChange}>
                <SelectTrigger id="authMethod">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="password">Password</SelectItem>
                  <SelectItem value="key">Private key</SelectItem>
                  <SelectItem value="agent">SSH agent</SelectItem>
                </SelectContent>
              </Select>
            )}
          />
        </div>
        {authMethod === 'key' && (
          <Field htmlFor="keyPath" label="Key path">
            <div className="flex gap-2">
              <Input id="keyPath" {...register('keyPath')} />
              <Button
                type="button"
                variant="secondary"
                onClick={async () => {
                  const picked = await open({ multiple: false, directory: false })
                  if (typeof picked === 'string') {
                    setValue('keyPath', picked, { shouldDirty: true })
                  }
                }}
              >
                Browse
              </Button>
            </div>
          </Field>
        )}
        {authMethod !== 'agent' && (
          <Field htmlFor="password" label="Password">
            <Input
              id="password"
              type="password"
              {...register('password')}
              placeholder={profile?.secretId ? 'unchanged' : ''}
            />
          </Field>
        )}
        <div className="space-y-1">
          <Label>Tunnels</Label>
          {fields.map((f, i) => {
            const kind = watch(`tunnels.${i}.kind`)
            const bindHost = watch(`tunnels.${i}.bindHost`)
            const exposed = bindHost !== '' && !['127.0.0.1', 'localhost', '::1'].includes(bindHost)
            return (
              <div
                key={f.id}
                className="flex flex-wrap items-center gap-1 rounded border border-border p-1"
              >
                <Controller
                  control={control}
                  name={`tunnels.${i}.kind`}
                  render={({ field }) => (
                    <Select value={field.value} onValueChange={field.onChange}>
                      <SelectTrigger className="h-8 w-24">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="local">Local</SelectItem>
                        <SelectItem value="dynamic">Dynamic</SelectItem>
                        <SelectItem value="remote">Remote</SelectItem>
                      </SelectContent>
                    </Select>
                  )}
                />
                <Input
                  className="h-8 w-24"
                  placeholder="bind host"
                  {...register(`tunnels.${i}.bindHost`)}
                />
                <Input
                  className="h-8 w-20"
                  type="number"
                  placeholder="port"
                  {...register(`tunnels.${i}.bindPort`, { valueAsNumber: true })}
                />
                {kind !== 'dynamic' && (
                  <>
                    <Input
                      className="h-8 w-24"
                      placeholder="target host"
                      {...register(`tunnels.${i}.targetHost`)}
                    />
                    <Input
                      className="h-8 w-20"
                      type="number"
                      placeholder="port"
                      {...register(`tunnels.${i}.targetPort`, { valueAsNumber: true })}
                    />
                  </>
                )}
                <label className="flex items-center gap-1 text-xs">
                  <input type="checkbox" {...register(`tunnels.${i}.autoStart`)} /> auto
                </label>
                <Button type="button" variant="ghost" size="sm" onClick={() => remove(i)}>
                  Remove
                </Button>
                {exposed && (
                  <p className="w-full text-destructive text-xs">
                    Binding to {bindHost} exposes this forward beyond localhost to the network.
                  </p>
                )}
              </div>
            )
          })}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() =>
              append({
                id: crypto.randomUUID(),
                kind: 'local',
                bindHost: '127.0.0.1',
                bindPort: 8080,
                targetHost: '',
                targetPort: 0,
                autoStart: false,
              })
            }
          >
            Add tunnel
          </Button>
        </div>
        <div className="space-y-1">
          <Label htmlFor="jumpHost">Jump host</Label>
          <Controller
            control={control}
            name="jumpHostId"
            render={({ field }) => (
              <Select
                value={field.value || 'none'}
                onValueChange={(v) => field.onChange(v === 'none' ? '' : v)}
              >
                <SelectTrigger id="jumpHost">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None</SelectItem>
                  {profiles
                    .filter((p) => p.id !== profile?.id)
                    .filter((p) => !wouldCycle(profiles, profile?.id ?? null, p.id))
                    .map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.name}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
            )}
          />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <Field htmlFor="appFontFamily" label="Font family override">
            <Input id="appFontFamily" placeholder="inherit" {...register('appFontFamily')} />
          </Field>
          <Field htmlFor="appFontSize" label="Font size override">
            <Input
              id="appFontSize"
              type="number"
              placeholder="inherit"
              {...register('appFontSize', { valueAsNumber: true })}
            />
          </Field>
        </div>
        <IconPicker value={iconRef} onChange={setIconRef} />
      </form>
    </PageShell>
  )
}

function Field({
  htmlFor,
  label,
  error,
  children,
}: {
  htmlFor: string
  label: string
  error?: string
  children: ReactNode
}) {
  return (
    <div className="space-y-1">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {error && <p className="text-red-600 text-xs">{error}</p>}
    </div>
  )
}
