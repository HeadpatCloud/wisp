import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { useEffect, useRef } from 'react'
import {
  type AppearanceSettings,
  applyTerminalSettings,
  resolveTerminalTheme,
} from './terminalTheme'

interface TerminalViewProps {
  onData: (data: string) => void
  onResize: (cols: number, rows: number) => void
  bindWrite: (write: (data: Uint8Array) => void) => void
  settings: AppearanceSettings
}

export function TerminalView({ onData, onResize, bindWrite, settings }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const initial = useRef(settings)

  useEffect(() => {
    if (termRef.current || !containerRef.current) return
    const s = initial.current
    const term = new Terminal({
      fontFamily: s.fontFamily,
      fontSize: s.fontSize,
      fontWeight: s.fontWeight as import('@xterm/xterm').FontWeight,
      lineHeight: s.lineHeight,
      letterSpacing: s.letterSpacing,
      cursorStyle: s.cursorStyle,
      cursorBlink: s.cursorBlink,
      scrollback: s.scrollback,
      theme: { ...resolveTerminalTheme(s.theme, s.colorScheme) },
    })
    termRef.current = term

    const fit = new FitAddon()
    fitRef.current = fit
    term.loadAddon(fit)
    term.loadAddon(new WebLinksAddon())
    term.open(containerRef.current)
    try {
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      term.loadAddon(webgl)
    } catch {
      // no WebGL: DOM renderer stays active
    }
    const dataSub = term.onData(onData)
    const resizeSub = term.onResize(({ cols, rows }) => onResize(cols, rows))
    bindWrite((data) => term.write(data))

    fit.fit()
    // fit's resize event isn't replayed to the listener above, so report the
    // initial fitted size explicitly - otherwise the PTY opens at 80x24.
    onResize(term.cols, term.rows)

    const ro = new ResizeObserver(() => fit.fit())
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      dataSub.dispose()
      resizeSub.dispose()
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
  }, [onData, onResize, bindWrite])

  useEffect(() => {
    if (termRef.current && fitRef.current) {
      applyTerminalSettings(termRef.current, fitRef.current, settings)
    }
  }, [settings])

  return <div ref={containerRef} className="h-full w-full" />
}
