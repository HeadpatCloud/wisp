import { render, waitFor } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'
import type { AppearanceSettings } from './terminalTheme'

const m = vi.hoisted(() => {
  const term = {
    open: vi.fn(),
    loadAddon: vi.fn(),
    write: vi.fn(),
    dispose: vi.fn(),
    getSelection: vi.fn(() => 'picked text'),
    clearSelection: vi.fn(),
    paste: vi.fn(),
    options: { fontFamily: 'monospace', fontSize: 14 } as {
      theme?: unknown
      fontFamily: string
      fontSize: number
    },
    onData: vi.fn((cb: (d: string) => void) => {
      term._data = cb
      return { dispose: vi.fn() }
    }),
    onResize: vi.fn((cb: (s: { cols: number; rows: number }) => void) => {
      term._resize = cb
      return { dispose: vi.fn() }
    }),
    onSelectionChange: vi.fn((cb: () => void) => {
      term._selection = cb
      return { dispose: vi.fn() }
    }),
    _data: (_: string) => {},
    _resize: (_: { cols: number; rows: number }) => {},
    _selection: () => {},
    cols: 80,
    rows: 24,
  }
  return { term, findNext: vi.fn(), findPrevious: vi.fn() }
})

vi.mock('@xterm/xterm', () => ({
  Terminal: vi.fn().mockImplementation(function (this: object) {
    Object.assign(this, m.term)
    Object.defineProperty(this, 'cols', { get: () => m.term.cols })
    Object.defineProperty(this, 'rows', { get: () => m.term.rows })
  }),
}))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: vi.fn().mockImplementation(function (this: { fit: ReturnType<typeof vi.fn> }) {
    this.fit = vi.fn(() => {
      m.term.cols = 120
      m.term.rows = 40
    })
  }),
}))
vi.mock('@xterm/addon-search', () => ({
  SearchAddon: vi.fn().mockImplementation(function (this: {
    findNext: ReturnType<typeof vi.fn>
    findPrevious: ReturnType<typeof vi.fn>
  }) {
    this.findNext = m.findNext
    this.findPrevious = m.findPrevious
  }),
}))
vi.mock('@xterm/addon-web-links', () => ({
  WebLinksAddon: vi.fn().mockImplementation(function (this: { dispose: ReturnType<typeof vi.fn> }) {
    this.dispose = vi.fn()
  }),
}))
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: vi.fn().mockImplementation(function (this: {
    onContextLoss: ReturnType<typeof vi.fn>
    dispose: ReturnType<typeof vi.fn>
  }) {
    this.onContextLoss = vi.fn()
    this.dispose = vi.fn()
  }),
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import { type TerminalSearch, TerminalView } from './TerminalView'

const settings: AppearanceSettings = {
  theme: 'dark',
  colorScheme: 'default',
  fontFamily: 'monospace',
  fontSize: 14,
  fontWeight: 'normal',
  lineHeight: 1,
  letterSpacing: 0,
  cursorStyle: 'block',
  cursorBlink: true,
  scrollback: 10000,
}

function renderView(props: Partial<React.ComponentProps<typeof TerminalView>> = {}) {
  return render(
    <TerminalView
      onData={vi.fn()}
      onResize={vi.fn()}
      bindWrite={() => {}}
      settings={settings}
      {...props}
    />,
  )
}

let observed: (() => void) | null = null

beforeEach(() => {
  vi.clearAllMocks()
  m.term.cols = 80
  m.term.rows = 24
  observed = null
  globalThis.ResizeObserver = class {
    constructor(cb: () => void) {
      observed = cb
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
  // As in a browser: nothing inside a display:none subtree has a box.
  vi.spyOn(HTMLElement.prototype, 'getClientRects').mockImplementation(function (
    this: HTMLElement,
  ) {
    return (this.closest('.hidden') ? [] : [new DOMRect(0, 0, 800, 600)]) as unknown as DOMRectList
  })
  Object.defineProperty(navigator, 'clipboard', {
    value: {
      writeText: vi.fn().mockResolvedValue(undefined),
      readText: vi.fn().mockResolvedValue('pasted'),
    },
    configurable: true,
  })
})

test('reports the fitted size via onResize on mount', () => {
  const onResize = vi.fn()
  renderView({ onResize })
  expect(onResize).toHaveBeenCalledWith(120, 40)
})

// A hidden tab's container reports "100%" as its size, which fit reads as 100 px.
test('a terminal in a hidden tab keeps its size until it is shown', () => {
  const onResize = vi.fn()
  const wrapper = document.createElement('div')
  wrapper.className = 'hidden'
  document.body.appendChild(wrapper)
  render(
    <TerminalView onData={vi.fn()} onResize={onResize} bindWrite={() => {}} settings={settings} />,
    {
      container: wrapper,
    },
  )
  expect(onResize).toHaveBeenCalledWith(80, 24)
  expect(onResize).not.toHaveBeenCalledWith(120, 40)

  observed?.()
  expect(m.term.cols).toBe(80)

  wrapper.className = ''
  observed?.()
  expect(m.term.cols).toBe(120)
  wrapper.remove()
})

test('opens, forwards data + resize, binds a write sink, disposes', () => {
  const onData = vi.fn()
  const onResize = vi.fn()
  let writeSink: ((d: Uint8Array) => void) | null = null
  const { unmount } = renderView({
    onData,
    onResize,
    bindWrite: (w) => {
      writeSink = w
    },
  })
  expect(m.term.open).toHaveBeenCalled()
  m.term._data('x')
  expect(onData).toHaveBeenCalledWith('x')
  m.term._resize({ cols: 100, rows: 30 })
  expect(onResize).toHaveBeenCalledWith(100, 30)
  ;(writeSink as ((d: Uint8Array) => void) | null)?.(new Uint8Array([104, 105]))
  expect(m.term.write).toHaveBeenCalled()
  unmount()
  expect(m.term.dispose).toHaveBeenCalled()
})

test('copies the selection only when copyOnSelect is on', () => {
  const { unmount } = renderView({ copyOnSelect: false })
  m.term._selection()
  expect(navigator.clipboard.writeText).not.toHaveBeenCalled()
  unmount()

  renderView({ copyOnSelect: true })
  m.term._selection()
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith('picked text')
})

// term.paste (not onData) so bracketed-paste framing is applied.
test('right-click pastes into the session only when rightClickPaste is on', async () => {
  const { container, unmount } = renderView({ rightClickPaste: false })
  container.firstChild?.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))
  expect(navigator.clipboard.readText).not.toHaveBeenCalled()
  unmount()

  const second = renderView({ rightClickPaste: true })
  second.container.firstChild?.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))
  await waitFor(() => expect(m.term.paste).toHaveBeenCalledWith('pasted'))
})

test('exposes a search handle and revokes it on unmount', () => {
  let api: TerminalSearch | null = null
  const { unmount } = renderView({
    bindSearch: (a) => {
      api = a
    },
  })
  ;(api as TerminalSearch | null)?.findNext('needle')
  expect(m.findNext).toHaveBeenCalledWith('needle')
  ;(api as TerminalSearch | null)?.findPrevious('needle')
  expect(m.findPrevious).toHaveBeenCalledWith('needle')
  unmount()
  expect(api).toBeNull()
})
