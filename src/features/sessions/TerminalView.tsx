import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
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

export interface TerminalSearch {
  findNext: (query: string) => void
  findPrevious: (query: string) => void
  clear: () => void
}

interface TerminalViewProps {
  onData: (data: string) => void
  onResize: (cols: number, rows: number) => void
  bindWrite: (write: (data: Uint8Array) => void) => void
  bindSearch?: (api: TerminalSearch | null) => void
  settings: AppearanceSettings
  copyOnSelect?: boolean
  rightClickPaste?: boolean
}

export function TerminalView({
  onData,
  onResize,
  bindWrite,
  bindSearch,
  settings,
  copyOnSelect,
  rightClickPaste,
}: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const initial = useRef(settings)
  // Read through refs so toggling these settings doesn't tear down the terminal.
  const copyRef = useRef(copyOnSelect)
  copyRef.current = copyOnSelect
  const pasteRef = useRef(rightClickPaste)
  pasteRef.current = rightClickPaste

  useEffect(() => {
    if (termRef.current || !containerRef.current) return
    const container = containerRef.current
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
    const search = new SearchAddon()
    term.loadAddon(search)
    term.open(container)
    try {
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      term.loadAddon(webgl)
    } catch {
      // no WebGL: DOM renderer stays active
    }
    const dataSub = term.onData(onData)
    const resizeSub = term.onResize(({ cols, rows }) => onResize(cols, rows))
    const selectionSub = term.onSelectionChange(() => {
      if (!copyRef.current) return
      const sel = term.getSelection()
      if (sel) navigator.clipboard.writeText(sel).catch(() => {})
    })
    // term.paste applies bracketed-paste framing and CRLF normalization; feeding onData
    // directly would send raw text the remote program can't distinguish from typing.
    const onContextMenu = (ev: MouseEvent) => {
      if (!pasteRef.current) return
      ev.preventDefault()
      navigator.clipboard
        .readText()
        .then((text) => text && term.paste(text))
        .catch(() => {})
    }
    container.addEventListener('contextmenu', onContextMenu)
    bindWrite((data) => term.write(data))
    bindSearch?.({
      findNext: (q) => search.findNext(q),
      findPrevious: (q) => search.findPrevious(q),
      clear: () => term.clearSelection(),
    })

    fit.fit()
    // fit's resize event isn't replayed to the listener above, so report the
    // initial fitted size explicitly - otherwise the PTY opens at 80x24.
    onResize(term.cols, term.rows)

    const ro = new ResizeObserver(() => fit.fit())
    ro.observe(container)

    return () => {
      ro.disconnect()
      container.removeEventListener('contextmenu', onContextMenu)
      dataSub.dispose()
      resizeSub.dispose()
      selectionSub.dispose()
      bindSearch?.(null)
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
  }, [onData, onResize, bindWrite, bindSearch])

  useEffect(() => {
    if (termRef.current && fitRef.current) {
      applyTerminalSettings(termRef.current, fitRef.current, settings)
    }
  }, [settings])

  return <div ref={containerRef} className="h-full w-full" />
}
