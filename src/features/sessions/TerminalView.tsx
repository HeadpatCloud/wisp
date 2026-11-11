import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { useEffect, useRef } from 'react'
import { guardedPaste } from '@/lib/tagRules'
import { confirm } from '@/stores/confirmStore'
import { type AppearanceSettings, applyTerminalSettings } from './terminalTheme'

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
  // Asked at paste time: whether text of several lines waits for the user (a tag rule).
  askBeforePaste?: () => boolean
}

// Long pastes show their start; the rest is counted.
function shownLines(text: string): string {
  const lines = text.split(/\r\n|\r|\n/)
  const head = lines.slice(0, 12).join('\n')
  return lines.length > 12 ? `${head}\n… and ${lines.length - 12} more lines` : head
}

export function TerminalView({
  onData,
  onResize,
  bindWrite,
  bindSearch,
  settings,
  copyOnSelect,
  rightClickPaste,
  askBeforePaste,
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
  const askRef = useRef(askBeforePaste)
  askRef.current = askBeforePaste

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
      theme: { ...s.colors },
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
    // Every paste comes through here, so a tag that asks first sees each one.
    const paste = (text: string) =>
      guardedPaste(
        text,
        askRef.current?.() ?? false,
        (lines) =>
          confirm({
            title: 'Paste these lines?',
            body: term.modes.bracketedPasteMode
              ? shownLines(lines)
              : `${shownLines(lines)}\n\nEach line runs as soon as it lands.`,
            confirmLabel: 'Paste',
          }),
        (t) => term.paste(t),
      )
    const onContextMenu = (ev: MouseEvent) => {
      if (!pasteRef.current) return
      ev.preventDefault()
      navigator.clipboard
        .readText()
        .then((text) => {
          if (text) void paste(text)
        })
        .catch(() => {})
    }
    // Ctrl+Shift+V and Shift+Insert reach xterm as a paste event on its textarea.
    const onPaste = (ev: ClipboardEvent) => {
      const text = ev.clipboardData?.getData('text/plain')
      if (!text || !askRef.current?.()) return
      ev.preventDefault()
      ev.stopPropagation()
      void paste(text)
    }
    container.addEventListener('contextmenu', onContextMenu)
    container.addEventListener('paste', onPaste, true)
    bindWrite((data) => term.write(data))
    bindSearch?.({
      findNext: (q) => search.findNext(q),
      findPrevious: (q) => search.findPrevious(q),
      clear: () => term.clearSelection(),
    })

    // In a hidden tab the computed size is "100%", which fit reads as 100 px and shrinks the
    // PTY to a few columns; the observer refits once the tab is shown.
    const refit = () => {
      if (container.getClientRects().length > 0) fit.fit()
    }
    refit()
    // fit's resize event isn't replayed to the listener above, so report the
    // initial fitted size explicitly - otherwise the PTY opens at 80x24.
    onResize(term.cols, term.rows)

    const ro = new ResizeObserver(refit)
    ro.observe(container)

    return () => {
      ro.disconnect()
      container.removeEventListener('contextmenu', onContextMenu)
      container.removeEventListener('paste', onPaste, true)
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

  // Clipped: after a font change the old grid briefly overflows, and the split panel's scrollbars
  // would shrink the size fit measures.
  return <div ref={containerRef} className="h-full w-full overflow-hidden" />
}
