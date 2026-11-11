import { open as pickFile } from '@tauri-apps/plugin-dialog'
import { X } from 'lucide-react'
import { useState } from 'react'
import { commands } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { unwrap } from '@/lib/ipc'
import { BUILT_IN_SCHEMES, parseItermColors, parseWindowsTerminal } from '@/lib/schemes'
import { useSettingsStore } from '@/stores/settingsStore'
import { useThemeStore } from '@/stores/themeStore'
import { Choice, Heading, Row, Toggle } from './controls'
import { TerminalPreview } from './TerminalPreview'

const errorText = (e: unknown) =>
  e && typeof e === 'object' && 'message' in e ? String(e.message) : String(e)

function NumberField({
  id,
  label,
  value,
  step,
  min,
  onChange,
}: {
  id: string
  label: string
  value: number
  step?: number
  min?: number
  onChange: (n: number) => void
}) {
  return (
    <Row id={id} label={label}>
      <Input
        id={id}
        type="number"
        step={step}
        value={value}
        onChange={(e) => {
          const n = Number(e.target.value)
          if (Number.isFinite(n) && (min === undefined || n >= min)) onChange(n)
        }}
      />
    </Row>
  )
}

export function TerminalPage() {
  const settings = useSettingsStore((s) => s.settings)
  const update = useSettingsStore((s) => s.update)
  const schemes = useThemeStore((s) => s.schemes)
  const addSchemes = useThemeStore((s) => s.addSchemes)
  const removeScheme = useThemeStore((s) => s.removeScheme)
  const [error, setError] = useState<string | null>(null)

  const importScheme = async () => {
    const path = await pickFile({
      multiple: false,
      filters: [{ name: 'Terminal colours', extensions: ['json', 'itermcolors'] }],
    })
    if (typeof path !== 'string') return
    const text = unwrap(await commands.readTextFile(path))
    const name = path.split(/[\\/]/).pop() ?? path
    await addSchemes(
      name.toLowerCase().endsWith('.itermcolors')
        ? [parseItermColors(text, name)]
        : parseWindowsTerminal(text),
    )
  }

  return (
    <div className="flex max-w-3xl flex-col gap-4">
      <TerminalPreview />
      <Heading>Colours</Heading>
      <Choice
        id="color-scheme"
        label="Colours"
        value={settings.colorScheme}
        onChange={(v) => update({ colorScheme: v })}
        options={[
          ['default', 'From the theme'],
          ...[...BUILT_IN_SCHEMES, ...schemes].map((s): [string, string] => [s.id, s.name]),
        ]}
      />
      {schemes.length > 0 && (
        <ul className="flex flex-col gap-1">
          {schemes.map((s) => (
            <li key={s.id} className="flex items-center gap-2 text-sm">
              <span className="min-w-0 flex-1 truncate">{s.name}</span>
              <button
                type="button"
                aria-label={`Remove ${s.name}`}
                onClick={() => {
                  if (settings.colorScheme === s.id) update({ colorScheme: 'default' })
                  removeScheme(s.id).catch((e: unknown) => setError(errorText(e)))
                }}
                className="rounded p-0.5 text-muted-foreground hover:bg-muted"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            setError(null)
            importScheme().catch((e: unknown) => setError(errorText(e)))
          }}
        >
          Import a scheme…
        </Button>
        <p className="mt-1 text-muted-foreground text-xs">
          A Windows Terminal scheme (.json) or an iTerm2 .itermcolors file.
        </p>
      </div>
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}

      <Heading>Text</Heading>
      <Row id="font-family" label="Font">
        <Input
          id="font-family"
          value={settings.fontFamily}
          onChange={(e) => update({ fontFamily: e.target.value })}
        />
      </Row>
      <div className="grid gap-3 sm:grid-cols-2">
        <NumberField
          id="font-size"
          label="Size"
          value={settings.fontSize}
          min={1}
          onChange={(n) => update({ fontSize: n })}
        />
        <Choice
          id="font-weight"
          label="Weight"
          value={settings.fontWeight ?? 'normal'}
          onChange={(v) => update({ fontWeight: v })}
          options={[
            ['normal', 'Normal'],
            ['medium', 'Medium'],
            ['bold', 'Bold'],
          ]}
        />
        <NumberField
          id="line-height"
          label="Line height"
          value={settings.lineHeight ?? 1}
          step={0.1}
          min={0.5}
          onChange={(n) => update({ lineHeight: n })}
        />
        <NumberField
          id="letter-spacing"
          label="Letter spacing"
          value={settings.letterSpacing ?? 0}
          step={0.5}
          onChange={(n) => update({ letterSpacing: n })}
        />
        <NumberField
          id="scrollback"
          label="Scrollback (lines)"
          value={settings.scrollback ?? 10000}
          min={0}
          onChange={(n) => update({ scrollback: n })}
        />
        <Choice
          id="cursor"
          label="Cursor"
          value={settings.cursorStyle ?? 'block'}
          onChange={(v) => update({ cursorStyle: v })}
          options={[
            ['block', 'Block'],
            ['bar', 'Bar'],
            ['underline', 'Underline'],
          ]}
        />
      </div>
      <Toggle
        label="Blink the cursor"
        checked={settings.cursorBlink ?? true}
        onChange={(v) => update({ cursorBlink: v })}
      />

      <Heading>Mouse</Heading>
      <Toggle
        label="Copy on select"
        checked={settings.copyOnSelect ?? false}
        onChange={(v) => update({ copyOnSelect: v })}
      />
      <Toggle
        label="Right-click pastes"
        checked={settings.rightClickPaste ?? false}
        onChange={(v) => update({ rightClickPaste: v })}
        hint="PuTTY-style mouse behaviour in terminals. Both off by default."
      />
    </div>
  )
}
