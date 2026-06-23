import { render } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => {
  const term = {
    open: vi.fn(),
    loadAddon: vi.fn(),
    write: vi.fn(),
    dispose: vi.fn(),
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
    _data: (_: string) => {},
    _resize: (_: { cols: number; rows: number }) => {},
    cols: 80,
    rows: 24,
  }
  return { term }
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

import { TerminalView } from './TerminalView'

beforeEach(() => {
  vi.clearAllMocks()
  m.term.cols = 80
  m.term.rows = 24
})

test('reports the fitted size via onResize on mount', () => {
  const onResize = vi.fn()
  render(
    <TerminalView
      onData={vi.fn()}
      onResize={onResize}
      bindWrite={() => {}}
      settings={{
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
      }}
    />,
  )
  expect(onResize).toHaveBeenCalledWith(120, 40)
})

test('opens, forwards data + resize, binds a write sink, disposes', () => {
  const onData = vi.fn()
  const onResize = vi.fn()
  let writeSink: ((d: Uint8Array) => void) | null = null
  const { unmount } = render(
    <TerminalView
      onData={onData}
      onResize={onResize}
      bindWrite={(w) => {
        writeSink = w
      }}
      settings={{
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
      }}
    />,
  )
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
