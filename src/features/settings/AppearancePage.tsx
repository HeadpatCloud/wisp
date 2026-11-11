import { open as pickFile, save as pickSave } from '@tauri-apps/plugin-dialog'
import { X } from 'lucide-react'
import { type CSSProperties, useState } from 'react'
import { commands } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { unwrap } from '@/lib/ipc'
import { ACCENTS } from '@/lib/theme'
import { BUILT_IN_THEMES, parseThemeFile, type Theme, themeFile } from '@/lib/themes'
import { cn } from '@/lib/utils'
import { useSettingsStore } from '@/stores/settingsStore'
import { activeTheme, useThemeStore } from '@/stores/themeStore'
import { Choice, Heading, Row, Toggle } from './controls'

const errorText = (e: unknown) =>
  e && typeof e === 'object' && 'message' in e ? String(e.message) : String(e)

// The card previews the theme in its own colours, which only it knows.
function ThemeCard({
  theme,
  selected,
  onPick,
  onRemove,
}: {
  theme: Theme
  selected: boolean
  onPick: () => void
  onRemove?: () => void
}) {
  const c = theme.colors
  return (
    <div
      className={cn(
        'relative flex flex-col gap-2 rounded-lg border p-2',
        selected ? 'border-ring ring-2 ring-ring' : 'border-border',
      )}
    >
      <button type="button" onClick={onPick} className="flex flex-col gap-2 text-left">
        <span
          className="flex h-14 items-end gap-1 rounded-md border p-1.5"
          style={
            {
              '--ui-hue': theme.tint,
              background: c.background,
              borderColor: c.border,
            } as CSSProperties
          }
        >
          <span className="h-full w-1/4 rounded-sm" style={{ background: c.sidebar }} />
          <span className="h-2 flex-1 rounded-sm" style={{ background: c.foreground }} />
          <span className="h-2 w-4 rounded-sm" style={{ background: c.primary }} />
        </span>
        <span className="truncate text-sm">{theme.name}</span>
      </button>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove ${theme.name}`}
          onClick={onRemove}
          className="absolute top-1 right-1 rounded p-0.5 text-muted-foreground hover:bg-muted"
        >
          <X className="size-3.5" />
        </button>
      )}
    </div>
  )
}

export function AppearancePage() {
  const settings = useSettingsStore((s) => s.settings)
  const update = useSettingsStore((s) => s.update)
  const userThemes = useThemeStore((s) => s.userThemes)
  const addTheme = useThemeStore((s) => s.addTheme)
  const removeTheme = useThemeStore((s) => s.removeTheme)
  const [error, setError] = useState<string | null>(null)
  const [otherFont, setOtherFont] = useState(
    !['inter', 'system'].includes(settings.uiFont ?? 'inter'),
  )
  const themes = [...BUILT_IN_THEMES, ...userThemes]
  const active = activeTheme(settings)
  const following = settings.theme !== 'light' && settings.theme !== 'dark'
  const lightId = settings.lightTheme ?? 'paper'
  const darkId = settings.darkTheme ?? 'graphite'

  const pick = (t: Theme) =>
    update(
      following
        ? t.kind === 'dark'
          ? { darkTheme: t.id }
          : { lightTheme: t.id }
        : { theme: t.kind, ...(t.kind === 'dark' ? { darkTheme: t.id } : { lightTheme: t.id }) },
    )

  const importTheme = async () => {
    const path = await pickFile({
      multiple: false,
      filters: [{ name: 'Wisp theme', extensions: ['json'] }],
    })
    if (typeof path !== 'string') return
    await addTheme(parseThemeFile(unwrap(await commands.readTextFile(path))))
  }
  const exportTheme = async () => {
    const path = await pickSave({
      defaultPath: `${active.name}.json`,
      filters: [{ name: 'Wisp theme', extensions: ['json'] }],
    })
    if (!path) return
    unwrap(await commands.writeTextFile(path, themeFile(active)))
  }
  const run = (f: () => Promise<void>) => {
    setError(null)
    f().catch((e: unknown) => setError(errorText(e)))
  }

  return (
    <div className="flex max-w-3xl flex-col gap-4">
      <Heading>Theme</Heading>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {themes.map((t) => (
          <ThemeCard
            key={t.id}
            theme={t}
            selected={following ? t.id === lightId || t.id === darkId : t.id === active.id}
            onPick={() => pick(t)}
            onRemove={t.builtIn ? undefined : () => run(() => removeTheme(t.id))}
          />
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" onClick={() => run(importTheme)}>
          Import a theme…
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={() => run(exportTheme)}>
          Export {active.name}…
        </Button>
      </div>
      <p className="text-muted-foreground text-xs">Export one to change it, then import it.</p>
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <Toggle
        label="Follow the system"
        checked={following}
        onChange={(on) => update({ theme: on ? 'system' : active.kind })}
      />
      {following && (
        <div className="grid gap-3 sm:grid-cols-2">
          <Choice
            id="light-uses"
            label="Light uses"
            value={lightId}
            onChange={(v) => update({ lightTheme: v })}
            options={themes.filter((t) => t.kind === 'light').map((t) => [t.id, t.name])}
          />
          <Choice
            id="dark-uses"
            label="Dark uses"
            value={darkId}
            onChange={(v) => update({ darkTheme: v })}
            options={themes.filter((t) => t.kind === 'dark').map((t) => [t.id, t.name])}
          />
        </div>
      )}

      <Heading>Interface</Heading>
      <Choice
        id="accent"
        label="Accent"
        value={settings.accent ?? 'teal'}
        onChange={(v) => update({ accent: v })}
        options={ACCENTS.map((a) => [
          a.value,
          <span key={a.value} className="flex items-center gap-2">
            <span
              className="size-3 rounded-full"
              style={{ background: `oklch(0.66 0.13 ${a.hue})` }}
            />
            {a.label}
          </span>,
        ])}
      />
      <Choice
        id="density"
        label="Density"
        value={settings.density ?? 'comfortable'}
        onChange={(v) => update({ density: v })}
        options={[
          ['comfortable', 'Comfortable'],
          ['compact', 'Compact'],
        ]}
      />
      <Choice
        id="ui-font"
        label="Interface font"
        value={otherFont ? 'other' : (settings.uiFont ?? 'inter')}
        onChange={(v) => {
          setOtherFont(v === 'other')
          if (v !== 'other') update({ uiFont: v })
        }}
        options={[
          ['inter', 'Inter'],
          ['system', 'System'],
          ['other', 'Another font…'],
        ]}
      />
      {otherFont && (
        <Row id="ui-font-name" label="Font family">
          <Input
            id="ui-font-name"
            value={
              ['inter', 'system'].includes(settings.uiFont ?? '') ? '' : (settings.uiFont ?? '')
            }
            onChange={(e) => update({ uiFont: e.target.value })}
          />
        </Row>
      )}
      <Choice
        id="host-list-side"
        label="Host list side"
        value={settings.hostListSide ?? 'left'}
        onChange={(v) => update({ hostListSide: v })}
        options={[
          ['left', 'Left'],
          ['right', 'Right'],
        ]}
      />
    </div>
  )
}
