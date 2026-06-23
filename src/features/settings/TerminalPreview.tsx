import { FitAddon } from '@xterm/addon-fit'
import { type FontWeight, Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { useEffect, useMemo, useRef } from 'react'
import { applyTerminalSettings, resolveTerminalTheme } from '@/features/sessions/terminalTheme'
import { appearanceOf, useSettingsStore } from '@/stores/settingsStore'

const SAMPLE = [
  '\x1b[1;32muser@remote\x1b[0m:\x1b[1;34m~/project\x1b[0m$ ls --color',
  '\x1b[1;34msrc\x1b[0m  \x1b[1;36mbuild.sh\x1b[0m  README.md  \x1b[1;31merror.log\x1b[0m',
  '\x1b[1;32muser@remote\x1b[0m:\x1b[1;34m~/project\x1b[0m$ echo "The quick brown fox 0123456789"',
  'The quick brown fox 0123456789',
  '\x1b[31m███\x1b[32m███\x1b[33m███\x1b[34m███\x1b[35m███\x1b[36m███\x1b[37m███\x1b[0m',
  '\x1b[90m███\x1b[91m███\x1b[92m███\x1b[93m███\x1b[94m███\x1b[95m███\x1b[96m███\x1b[97m███\x1b[0m',
].join('\r\n')

export function TerminalPreview() {
  const raw = useSettingsStore((s) => s.settings)
  const settings = useMemo(() => appearanceOf(raw), [raw])
  const initial = useRef(settings)
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)

  useEffect(() => {
    if (termRef.current || !containerRef.current) return
    const s = initial.current
    const term = new Terminal({
      fontFamily: s.fontFamily,
      fontSize: s.fontSize,
      fontWeight: s.fontWeight as FontWeight,
      lineHeight: s.lineHeight,
      letterSpacing: s.letterSpacing,
      cursorStyle: s.cursorStyle,
      cursorBlink: s.cursorBlink,
      scrollback: 0,
      disableStdin: true,
      convertEol: true,
      theme: { ...resolveTerminalTheme(s.theme, s.colorScheme) },
    })
    termRef.current = term
    const fit = new FitAddon()
    fitRef.current = fit
    term.loadAddon(fit)
    term.open(containerRef.current)
    fit.fit()
    term.write(SAMPLE)
    return () => {
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
  }, [])

  useEffect(() => {
    if (termRef.current && fitRef.current) {
      applyTerminalSettings(termRef.current, fitRef.current, settings)
    }
  }, [settings])

  return (
    <div className="space-y-1">
      <span className="font-medium text-sm">Preview</span>
      <div
        ref={containerRef}
        className="h-40 w-full overflow-hidden rounded border border-border p-1"
      />
    </div>
  )
}
