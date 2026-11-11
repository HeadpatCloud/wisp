import { RotateCcw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Input } from '@/components/ui/input'
import {
  chordFor,
  chordOf,
  conflictFor,
  formatChord,
  HOTKEY_ACTIONS,
  type HotkeyAction,
  suspendHotkeys,
} from '@/lib/hotkeys'
import { cn } from '@/lib/utils'
import { useSettingsStore } from '@/stores/settingsStore'

export function KeyboardPage() {
  const settings = useSettingsStore((s) => s.settings)
  const update = useSettingsStore((s) => s.update)
  const [capturing, setCapturing] = useState<HotkeyAction | null>(null)
  const [bindError, setBindError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const overrides = settings.hotkeys ?? {}

  useEffect(() => {
    if (!capturing) return
    suspendHotkeys(true)
    setBindError(null)
    const onKey = (ev: KeyboardEvent) => {
      if (['Control', 'Shift', 'Alt', 'Meta'].includes(ev.key)) return
      ev.preventDefault()
      ev.stopPropagation()
      if (ev.key === 'Escape') {
        setCapturing(null)
        return
      }
      // Without a modifier the key would be swallowed before the terminal ever sees it.
      if (!ev.ctrlKey && !ev.altKey && !ev.metaKey) {
        setBindError('Shortcuts need Ctrl, Alt or Cmd.')
        return
      }
      const chord = chordOf(ev)
      const clash = conflictFor(capturing, chord, settings.hotkeys ?? {})
      if (clash) {
        setBindError(
          `${formatChord(chord)} is already used by "${HOTKEY_ACTIONS.find((a) => a.id === clash)?.label}".`,
        )
        return
      }
      update({ hotkeys: { ...(settings.hotkeys ?? {}), [capturing]: chord } })
      setCapturing(null)
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      suspendHotkeys(false)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [capturing, settings.hotkeys, update])

  const q = query.trim().toLowerCase()
  const shown = HOTKEY_ACTIONS.filter(
    (a) => !q || `${a.label} ${formatChord(chordFor(a.id, overrides))}`.toLowerCase().includes(q),
  )

  return (
    <div className="flex max-w-2xl flex-col gap-3">
      <Input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Filter"
        aria-label="Filter shortcuts"
      />
      <p className="text-muted-foreground text-xs">
        Click a shortcut to record new keys. Esc cancels.
      </p>
      {bindError && <p className="text-destructive text-xs">{bindError}</p>}
      <div className="flex flex-col">
        {shown.map((a) => (
          <div key={a.id} className="flex items-center gap-2 py-1 text-sm">
            <span className="min-w-0 flex-1 truncate">{a.label}</span>
            <button
              type="button"
              onClick={() => setCapturing(a.id)}
              className={cn(
                'rounded border border-border px-2 py-1 font-mono text-xs hover:bg-muted',
                capturing === a.id && 'border-ring text-muted-foreground',
              )}
            >
              {capturing === a.id ? 'Press keys…' : formatChord(chordFor(a.id, overrides))}
            </button>
            {overrides[a.id] && (
              <button
                type="button"
                aria-label={`Reset ${a.label}`}
                onClick={() => {
                  const next = { ...overrides }
                  delete next[a.id]
                  update({ hotkeys: next })
                }}
                className="rounded p-1 text-muted-foreground hover:bg-muted"
              >
                <RotateCcw className="size-3.5" />
              </button>
            )}
          </div>
        ))}
      </div>
      <p className="text-muted-foreground text-xs">
        Plain Ctrl+letter keys stay with the shell while a terminal has focus.
      </p>
      <p className="text-muted-foreground text-xs">
        On macOS the palette opens with Cmd K everywhere, terminals included.
      </p>
    </div>
  )
}
