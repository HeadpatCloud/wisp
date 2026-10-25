import { act, fireEvent, render, renderHook, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { StrictMode } from 'react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { suspendHotkeys, useHotkeys } from '@/lib/hotkeys'
import type { LockKeys, RemoteDriver, RemoteSession, Viewport } from '@/lib/remoteDriver'
import type { FrameMessage } from '@/lib/remoteFrames'
import { trustHostKey } from '@/lib/ssh'
import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { RemoteDesktopView } from './RemoteDesktopView'

vi.mock('@/lib/ssh', () => ({ trustHostKey: vi.fn() }))
vi.mock('@/lib/vnc', () => {
  throw new Error('the view must not load lib/vnc')
})

const WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'
const PICTURE_ERROR = 'The picture could not be updated.'

class FakeImageData {
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
}

const ctx = { putImageData: vi.fn(), drawImage: vi.fn() }
const removeTab = vi.fn()
const clipboard = { readText: vi.fn(), writeText: vi.fn() }
const setPointerCapture = vi.fn()
const requestFullscreen = vi.fn()
const exitFullscreen = vi.fn()
let fullscreenElement: Element | null = null
let localText = 'local'
let heldWrites: (() => void)[] | null = null
let frames: FrameRequestCallback[] = []
let area = { width: 0, height: 0 }
let ratioQueries: { media: string; listeners: Set<() => void> }[] = []
let observers: { run: () => void; observed: Element[]; disconnected: boolean }[] = []

function setClipboardSync(on: boolean) {
  useSettingsStore.setState({
    settings: { ...useSettingsStore.getState().settings, vncClipboardSync: on },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  ctx.putImageData.mockReset()
  frames = []
  area = { width: 0, height: 0 }
  observers = []
  ratioQueries = []
  fullscreenElement = null
  vi.stubGlobal('ImageData', FakeImageData)
  vi.stubGlobal(
    'ResizeObserver',
    class {
      private entry = { run: () => {}, observed: [] as Element[], disconnected: false }
      constructor(run: () => void) {
        this.entry.run = run
        observers.push(this.entry)
      }
      observe(element: Element) {
        this.entry.observed.push(element)
      }
      unobserve() {}
      disconnect() {
        this.entry.disconnected = true
      }
    },
  )
  vi.stubGlobal('matchMedia', (media: string) => {
    const query = { media, matches: false, listeners: new Set<() => void>() }
    ratioQueries.push(query)
    return {
      media,
      matches: false,
      addEventListener: (_type: string, run: () => void) => query.listeners.add(run),
      removeEventListener: (_type: string, run: () => void) => query.listeners.delete(run),
    }
  })
  const sizeOf = (side: 'width' | 'height') =>
    function (this: Element) {
      // A browser gives an element with display: none, or inside one, no size.
      if (this.closest('.hidden')) return 0
      return this.firstElementChild instanceof HTMLCanvasElement ? area[side] : 0
    }
  vi.spyOn(Element.prototype, 'clientWidth', 'get').mockImplementation(sizeOf('width'))
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(sizeOf('height'))
  vi.stubGlobal('requestAnimationFrame', (run: FrameRequestCallback) => frames.push(run))
  vi.stubGlobal('cancelAnimationFrame', (handle: number) => {
    if (handle > 0) frames[handle - 1] = () => {}
  })
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as never)
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;base64,AA')
  Element.prototype.setPointerCapture = setPointerCapture
  Element.prototype.requestFullscreen = requestFullscreen.mockResolvedValue(undefined)
  document.exitFullscreen = exitFullscreen.mockResolvedValue(undefined)
  Object.defineProperty(document, 'fullscreenElement', {
    configurable: true,
    get: () => fullscreenElement,
  })
  localText = 'local'
  heldWrites = null
  clipboard.readText.mockReset()
  clipboard.writeText.mockReset()
  clipboard.readText.mockImplementation(async () => localText)
  clipboard.writeText.mockImplementation(
    (text: string) =>
      new Promise<void>((resolve) => {
        const land = () => {
          localText = text
          resolve()
        }
        if (heldWrites) heldWrites.push(land)
        else land()
      }),
  )
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: clipboard })
  vi.mocked(trustHostKey).mockResolvedValue(undefined)
  useSessionStore.setState({ removeTab } as never)
  setClipboardSync(false)
})

afterEach(() => {
  for (const button of document.querySelectorAll('body > button')) button.remove()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  suspendHotkeys(false)
})

function fakeDriver(rdp = false) {
  const resize = vi.fn(async (_id: string, _viewport: Viewport) => {})
  const locks = vi.fn(async (_id: string, _locks: LockKeys) => {})
  const opens: {
    onFrame: (m: FrameMessage) => void
    signal: AbortSignal | undefined
    resolve: (session: RemoteSession) => void
    reject: (error: unknown) => void
  }[] = []
  const driver = {
    open: vi.fn(
      (onFrame: (m: FrameMessage) => void, _viewport: Viewport, signal?: AbortSignal) =>
        new Promise<RemoteSession>((resolve, reject) => {
          opens.push({ onFrame, signal, resolve, reject })
        }),
    ),
    pointer: vi.fn(async () => {}),
    key: vi.fn(async () => {}),
    clipboard: vi.fn(async () => {}),
    ack: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    ...(rdp ? { resize, locks, scancodes: true } : {}),
  } satisfies RemoteDriver
  return { driver, opens, resize, locks }
}

function session(id = 's1'): RemoteSession {
  return { id, width: 200, height: 100, name: 'desk' }
}

function start(active = true, rdp = false) {
  const fake = fakeDriver(rdp)
  const view = render(<RemoteDesktopView tabId="t1" driver={fake.driver} active={active} />)
  const canvas = view.container.querySelector('canvas') as HTMLCanvasElement
  const open = (opened = session()) =>
    act(async () => {
      fake.opens[fake.opens.length - 1].resolve(opened)
    })
  const fail = (error: unknown) =>
    act(async () => {
      fake.opens[fake.opens.length - 1].reject(error)
    })
  const emit = (...messages: FrameMessage[]) =>
    act(async () => {
      for (const m of messages) fake.opens[fake.opens.length - 1].onFrame(m)
    })
  const setActive = (value: boolean) =>
    act(async () => {
      view.rerender(<RemoteDesktopView tabId="t1" driver={fake.driver} active={value} />)
    })
  return { ...fake, ...view, canvas, open, fail, emit, setActive }
}

async function connect(active = true, rdp = false) {
  const view = start(active, rdp)
  await view.open()
  return view
}

function pixels(w: number, h: number): Uint8ClampedArray<ArrayBuffer> {
  return new Uint8ClampedArray(w * h * 4)
}

function rect(x: number, y: number, w: number, h: number): FrameMessage {
  return { kind: 'rect', x, y, w, h, rgba: pixels(w, h) }
}

function displayAt(canvas: HTMLCanvasElement, left: number, top: number, w: number, h: number) {
  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue({
    left,
    top,
    width: w,
    height: h,
    right: left + w,
    bottom: top + h,
    x: left,
    y: top,
    toJSON: () => ({}),
  })
}

async function connectSynced() {
  setClipboardSync(true)
  const view = await connect()
  await act(async () => view.canvas.focus())
  await act(async () => view.canvas.blur())
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'local']])
  view.driver.clipboard.mockClear()
  clipboard.readText.mockClear()
  clipboard.writeText.mockClear()
  return view
}

async function focusAgain(canvas: HTMLCanvasElement) {
  await act(async () => canvas.blur())
  await act(async () => canvas.focus())
}

async function landWrites(count: number) {
  await act(async () => {
    for (const land of heldWrites?.splice(0, count) ?? []) land()
  })
}

function outsideButton() {
  const button = document.createElement('button')
  document.body.appendChild(button)
  return button
}

// An exit animation keeps a dialog's content mounted after it has closed.
function keepClosingDialogs() {
  const computed = window.getComputedStyle.bind(window)
  vi.stubGlobal('getComputedStyle', (el: Element, pseudo?: string) => {
    const styles = computed(el, pseudo)
    if (!el.getAttribute('data-slot')?.startsWith('dialog-')) return styles
    return new Proxy(styles, {
      get(target, prop) {
        if (prop === 'animationName') {
          return el.getAttribute('data-state') === 'open' ? 'enter' : 'exit'
        }
        const value = Reflect.get(target, prop)
        return typeof value === 'function' ? value.bind(target) : value
      },
    })
  })
}

function resizeTo(width: number, height: number) {
  area = { width, height }
  act(() => {
    for (const observer of observers) if (!observer.disconnected) observer.run()
  })
}

function changeRatio(ratio: number) {
  vi.stubGlobal('devicePixelRatio', ratio)
  act(() => {
    for (const query of [...ratioQueries]) for (const run of [...query.listeners]) run()
  })
}

function ratioListeners() {
  return ratioQueries.reduce((count, query) => count + query.listeners.size, 0)
}

async function wait(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms)
  })
}

function useFakeTimers() {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
}

function runFrames() {
  for (const run of frames.splice(0)) run(0)
}

function setFullscreen(element: Element | null) {
  fullscreenElement = element
  act(() => {
    document.dispatchEvent(new Event('fullscreenchange'))
  })
}

// The frontend is compiled without Node's types, and jsdom has no unhandledrejection event.
const node = globalThis as unknown as {
  process: {
    on(event: 'unhandledRejection', run: () => void): void
    off(event: 'unhandledRejection', run: () => void): void
  }
}

async function unhandledRejections(run: () => Promise<void>) {
  const seen = vi.fn()
  node.process.on('unhandledRejection', seen)
  await run()
  await new Promise((resolve) => setTimeout(resolve))
  node.process.off('unhandledRejection', seen)
  return seen.mock.calls.length
}

function keys(driver: { key: { mock: { calls: unknown[][] } } }) {
  return driver.key.mock.calls.map(([, down, keysym]) => [down, keysym])
}

function points(driver: { pointer: { mock: { calls: unknown[][] } } }) {
  return driver.pointer.mock.calls.map(([, buttons, x, y]) => [buttons, x, y])
}

test('shows Connecting…, then the canvas with the opened size', async () => {
  const view = start()
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull()

  await view.open()

  expect(screen.queryByText('Connecting…')).toBeNull()
  expect(view.canvas.parentElement).not.toHaveClass('invisible')
  expect([view.canvas.width, view.canvas.height]).toEqual([200, 100])
  expect(screen.queryByText('desk')).toBeNull()
})

test('a rect is painted at its position with its pixels', async () => {
  const view = await connect()
  const rgba = pixels(2, 5)
  await view.emit({ kind: 'rect', x: 3, y: 4, w: 2, h: 5, rgba })

  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  const [image, x, y] = ctx.putImageData.mock.calls[0]
  expect([x, y]).toEqual([3, 4])
  expect(image).toBeInstanceOf(FakeImageData)
  expect([image.width, image.height]).toEqual([2, 5])
  expect(image.data).toBe(rgba)
})

test('an empty rect paints nothing and keeps the session', async () => {
  const view = await connect()
  await view.emit(rect(3, 4, 0, 0), rect(0, 0, 0, 7), rect(0, 0, 7, 0))

  expect(ctx.putImageData).not.toHaveBeenCalled()
  expect(view.driver.close).not.toHaveBeenCalled()
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
})

test('a copy draws the canvas onto itself from the source rectangle', async () => {
  const view = await connect()
  await view.emit({ kind: 'copy', x: 1, y: 2, w: 30, h: 40, srcX: 5, srcY: 6 })

  expect(ctx.drawImage).toHaveBeenCalledWith(view.canvas, 5, 6, 30, 40, 1, 2, 30, 40)
})

test('a resize changes the canvas size', async () => {
  const view = await connect()
  await view.emit({ kind: 'resize', w: 640, h: 480 })

  expect([view.canvas.width, view.canvas.height]).toEqual([640, 480])
})

test('messages that arrive before open resolves are applied afterwards in order', async () => {
  const view = start()
  await view.emit(
    { kind: 'resize', w: 300, h: 150 },
    rect(1, 1, 1, 1),
    { kind: 'copy', x: 0, y: 0, w: 1, h: 1, srcX: 1, srcY: 1 },
    rect(2, 2, 1, 1),
  )
  expect(ctx.putImageData).not.toHaveBeenCalled()
  expect(ctx.drawImage).not.toHaveBeenCalled()

  await view.open()

  expect([view.canvas.width, view.canvas.height]).toEqual([300, 150])
  expect(ctx.putImageData.mock.calls.map(([, x, y]) => [x, y])).toEqual([
    [1, 1],
    [2, 2],
  ])
  const [first, second] = ctx.putImageData.mock.invocationCallOrder
  const [copy] = ctx.drawImage.mock.invocationCallOrder
  expect(first).toBeLessThan(copy)
  expect(copy).toBeLessThan(second)
})

test('each sync is acknowledged once, after the rect before it was painted', async () => {
  const view = await connect()
  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' })

  expect(view.driver.ack.mock.calls).toEqual([['s1']])
  expect(ctx.putImageData.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.ack.mock.invocationCallOrder[0],
  )

  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' })
  expect(view.driver.ack.mock.calls).toEqual([['s1'], ['s1']])
  expect(ctx.putImageData.mock.invocationCallOrder[1]).toBeLessThan(
    view.driver.ack.mock.invocationCallOrder[1],
  )
})

test('a sync queued before open resolves is acknowledged once afterwards', async () => {
  const view = start()
  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' })
  expect(view.driver.ack).not.toHaveBeenCalled()

  await view.open()

  expect(view.driver.ack.mock.calls).toEqual([['s1']])
  expect(ctx.putImageData.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.ack.mock.invocationCallOrder[0],
  )
})

test('nothing is acknowledged without a sync', async () => {
  const view = await connect()
  await view.emit(
    rect(0, 0, 1, 1),
    { kind: 'copy', x: 0, y: 0, w: 1, h: 1, srcX: 1, srcY: 1 },
    { kind: 'resize', w: 10, h: 10 },
    { kind: 'cursor', hotX: 0, hotY: 0, w: 0, h: 0, rgba: pixels(0, 0) },
    { kind: 'clipboard', text: 'x' },
  )

  expect(view.driver.ack).not.toHaveBeenCalled()
})

test('a failed acknowledgement is not shown as an error', async () => {
  const view = await connect()
  // A plain function: a mock handles the promises it returns, which would hide a missing catch.
  Object.assign(view.driver, { ack: () => Promise.reject(new Error('gone')) })

  expect(await unhandledRejections(() => view.emit({ kind: 'sync' }))).toBe(0)
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
  expect(screen.queryByText(/gone/)).toBeNull()
})

test('closed shows the reason with Reconnect and Close tab', async () => {
  const view = await connect()
  await view.emit({ kind: 'closed', reason: 'network connection lost' })

  expect(screen.getByText('network connection lost')).toBeInTheDocument()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull()
  expect(view.driver.close).not.toHaveBeenCalled()

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(screen.getByText('Connecting…')).toBeInTheDocument()

  await view.open(session('s2'))
  expect(screen.queryByText('network connection lost')).toBeNull()
  expect(view.canvas.parentElement).not.toHaveClass('invisible')

  await view.emit({ kind: 'closed', reason: 'bye' })
  fireEvent.click(screen.getByRole('button', { name: 'Close tab' }))
  expect(removeTab).toHaveBeenCalledWith('t1')
})

test('messages after closed are ignored', async () => {
  const view = await connect()
  await view.emit({ kind: 'closed', reason: 'bye' }, rect(0, 0, 1, 1), { kind: 'sync' })

  expect(ctx.putImageData).not.toHaveBeenCalled()
  expect(view.driver.ack).not.toHaveBeenCalled()
})

test('a closed message queued before open resolves ends the session once it opens', async () => {
  const view = start()
  await view.emit(
    rect(0, 0, 1, 1),
    { kind: 'closed', reason: 'too many clients' },
    { kind: 'sync' },
  )
  await view.open()

  expect(screen.getByText('too many clients')).toBeInTheDocument()
  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  expect(view.driver.ack).not.toHaveBeenCalled()
})

test('an unknown certificate can be trusted, which connects again', async () => {
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })

  expect(screen.getByRole('heading', { name: 'Unknown certificate' })).toBeInTheDocument()
  expect(screen.getByText('h:5900')).toBeInTheDocument()
  expect(screen.getByText('SHA256:ab')).toBeInTheDocument()
  expect(screen.queryByText('Connecting…')).toBeNull()

  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })

  expect(trustHostKey).toHaveBeenCalledWith('vnc/h', 5900, 'SHA256:ab')
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(screen.queryByRole('heading', { name: 'Unknown certificate' })).toBeNull()

  await view.open()
  expect(view.canvas.parentElement).not.toHaveClass('invisible')
})

test('rejecting an unknown certificate shows Certificate rejected.', async () => {
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Reject' }))

  expect(screen.getByText('Certificate rejected.')).toBeInTheDocument()
  expect(trustHostKey).not.toHaveBeenCalled()
  expect(view.driver.open).toHaveBeenCalledTimes(1)
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Close tab' })).toBeInTheDocument()
})

test('Reject on the closing certificate dialog does not end the attempt that Trust started', async () => {
  keepClosingDialogs()
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
    'data-state',
    'closed',
  )

  fireEvent.click(screen.getByRole('button', { name: 'Reject' }))

  expect(screen.queryByText('Certificate rejected.')).toBeNull()
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
  await view.open()
  expect(view.canvas.parentElement).not.toHaveClass('invisible')
})

test('a changed certificate is trusted with the offered fingerprint', async () => {
  const view = start()
  await view.fail({
    kind: 'hostKeyMismatch',
    message: { host: 'vnc/h', port: 5901, stored: 'SHA256:old', offered: 'SHA256:new' },
  })

  expect(screen.getByRole('heading', { name: 'Certificate CHANGED' })).toBeInTheDocument()
  expect(screen.getByText('stored: SHA256:old')).toBeInTheDocument()
  expect(screen.getByText('offered: SHA256:new')).toBeInTheDocument()

  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Accept changed certificate' }))
  })

  expect(trustHostKey).toHaveBeenCalledWith('vnc/h', 5901, 'SHA256:new')
  expect(view.driver.open).toHaveBeenCalledTimes(2)
})

test.each([
  [
    'Trust',
    { kind: 'hostKeyUnknown', message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' } },
    'Unknown certificate',
    'Trust',
  ],
  [
    'Reject',
    { kind: 'hostKeyUnknown', message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' } },
    'Unknown certificate',
    'Trust',
  ],
  [
    'Accept changed certificate',
    {
      kind: 'hostKeyMismatch',
      message: { host: 'vnc/h', port: 5901, stored: 'SHA256:old', offered: 'SHA256:new' },
    },
    'Certificate CHANGED',
    'Accept changed certificate',
  ],
])(
  'the certificate dialog keeps its wording while it closes after %s',
  async (button, error, title, accept) => {
    keepClosingDialogs()
    const view = start()
    await view.fail(error)
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: button }))
    })

    expect(document.querySelector('[data-slot="dialog-content"]')).toHaveAttribute(
      'data-state',
      'closed',
    )
    expect(screen.getByRole('heading', { name: title })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: accept })).toBeInTheDocument()
    expect(
      screen.getByText(`${error.message.host.replace('vnc/', '')}:${error.message.port}`),
    ).toBeInTheDocument()
  },
)

test('a certificate that cannot be stored shows why and does not connect', async () => {
  vi.mocked(trustHostKey).mockRejectedValue(new Error('io: disk full'))
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })

  expect(screen.getByText('io: disk full')).toBeInTheDocument()
  expect(view.driver.open).toHaveBeenCalledTimes(1)
})

test.each([
  [{ kind: 'internal', message: 'vnc: wrong password' }, 'vnc: wrong password'],
  [{ kind: 'internal', message: 'the server offers only: 5, 16' }, 'the server offers only: 5, 16'],
  [{ kind: 'crypto' }, 'crypto'],
  [new Error('boom'), 'Error: boom'],
])('a connect error %j shows %s with Reconnect and Close tab', async (error, text) => {
  const view = start()
  await view.fail(error)

  expect(screen.getByText(text)).toBeInTheDocument()
  expect(screen.queryByText('Connecting…')).toBeNull()

  fireEvent.click(screen.getByRole('button', { name: 'Close tab' }))
  expect(removeTab).toHaveBeenCalledWith('t1')

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(view.driver.close).not.toHaveBeenCalled()
})

test('a long error or reason wraps and scrolls inside the overlay', async () => {
  const long = 'x'.repeat(5000)
  const view = start()
  await view.fail({ kind: 'io', message: long })
  expect(screen.getByText(long)).toHaveClass('break-words', 'max-h-40', 'overflow-y-auto')

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  await view.open()
  await view.emit({ kind: 'closed', reason: long })
  expect(screen.getByText(long)).toHaveClass('break-words', 'max-h-40', 'overflow-y-auto')
})

test('a canvas without a 2d context fails without connecting', () => {
  vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null)
  const view = start()

  expect(screen.getByText(PICTURE_ERROR)).toBeInTheDocument()
  expect(view.driver.open).not.toHaveBeenCalled()
})

test('Shift+A releases the keysym that was pressed', async () => {
  const view = await connect()
  expect(fireEvent.keyDown(view.canvas, { code: 'ShiftLeft', key: 'Shift' })).toBe(false)
  expect(fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'A' })).toBe(false)
  expect(fireEvent.keyUp(view.canvas, { code: 'ShiftLeft', key: 'Shift' })).toBe(false)
  expect(fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'a' })).toBe(false)

  expect(view.driver.key.mock.calls).toEqual([
    ['s1', true, 0xffe1, 'ShiftLeft'],
    ['s1', true, 0x41, 'KeyA'],
    ['s1', false, 0xffe1, 'ShiftLeft'],
    ['s1', false, 0x41, 'KeyA'],
  ])
})

test('a held key repeats its keysym', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', repeat: true })

  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [true, 0x61],
  ])
})

test('a key without a keysym is left to the browser', async () => {
  const view = await connect()
  const seen = vi.fn()
  window.addEventListener('keydown', seen)
  expect(fireEvent.keyDown(view.canvas, { code: 'BracketLeft', key: 'Dead' })).toBe(true)
  window.removeEventListener('keydown', seen)

  expect(view.driver.key).not.toHaveBeenCalled()
  expect(seen).toHaveBeenCalledTimes(1)
})

test('keys the canvas consumes do not reach listeners on the window', async () => {
  const view = await connect()
  const seen = vi.fn()
  window.addEventListener('keydown', seen)
  window.addEventListener('keyup', seen)
  fireEvent.keyDown(view.canvas, { code: 'Backspace', key: 'Backspace' })
  fireEvent.keyUp(view.canvas, { code: 'Backspace', key: 'Backspace' })
  window.removeEventListener('keydown', seen)
  window.removeEventListener('keyup', seen)

  expect(seen).not.toHaveBeenCalled()
  expect(keys(view.driver)).toEqual([
    [true, 0xff08],
    [false, 0xff08],
  ])
})

test('global hotkeys do not fire for keys pressed on the canvas', async () => {
  const closeTab = vi.fn()
  renderHook(() => useHotkeys({ closeTab }, {}))
  const view = await connect()
  const chord = { code: 'KeyW', key: 'W', ctrlKey: true, shiftKey: true }

  fireEvent.keyDown(view.canvas, chord)
  expect(closeTab).not.toHaveBeenCalled()
  expect(keys(view.driver)).toEqual([[true, 0x57]])

  fireEvent.keyDown(document.body, chord)
  expect(closeTab).toHaveBeenCalledTimes(1)
})

test('focus and blur leave a hotkey suspension set elsewhere alone', async () => {
  const closeTab = vi.fn()
  renderHook(() => useHotkeys({ closeTab }, {}))
  const view = await connect()
  const chord = { code: 'KeyW', key: 'W', ctrlKey: true, shiftKey: true }

  suspendHotkeys(true)
  act(() => view.canvas.focus())
  act(() => view.canvas.blur())
  fireEvent.keyDown(document.body, chord)
  expect(closeTab).not.toHaveBeenCalled()

  suspendHotkeys(false)
  act(() => view.canvas.focus())
  fireEvent.keyDown(document.body, chord)
  expect(closeTab).toHaveBeenCalledTimes(1)
})

test('a key pressed during composition is neither sent nor consumed', async () => {
  const view = await connect()
  const seen = vi.fn()
  window.addEventListener('keydown', seen)
  window.addEventListener('keyup', seen)
  expect(fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', isComposing: true })).toBe(true)
  expect(fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'a', isComposing: true })).toBe(true)
  window.removeEventListener('keydown', seen)
  window.removeEventListener('keyup', seen)

  expect(view.driver.key).not.toHaveBeenCalled()
  expect(seen).toHaveBeenCalledTimes(2)
})

test('the canvas is labelled, marked for the hotkey handler and shows its focus', async () => {
  const view = await connect()

  expect(screen.getByLabelText('Remote desktop')).toBe(view.canvas)
  expect(view.canvas).toHaveAttribute('data-remote-desktop')
  expect(view.canvas).toHaveClass('touch-none')
  expect(view.canvas.parentElement).toHaveClass('border-transparent', 'focus-within:border-ring')
})

test('blur releases everything held', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  fireEvent.keyDown(view.canvas, { code: 'ControlLeft', key: 'Control' })
  fireEvent.keyDown(view.canvas, { code: 'KeyC', key: 'c' })
  view.driver.key.mockClear()

  act(() => view.canvas.blur())

  expect(keys(view.driver)).toEqual([
    [false, 0xffe3],
    [false, 0x63],
  ])
  fireEvent.keyUp(view.canvas, { code: 'KeyC', key: 'c' })
  expect(view.driver.key).toHaveBeenCalledTimes(2)
})

test('the tab becoming inactive releases everything held and takes focus off the canvas', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  fireEvent.keyDown(view.canvas, { code: 'AltLeft', key: 'Alt' })
  fireEvent.keyDown(view.canvas, { code: 'Tab', key: 'Tab' })
  view.driver.key.mockClear()

  await view.setActive(false)

  expect(keys(view.driver)).toEqual([
    [false, 0xffe9],
    [false, 0xff09],
  ])
  expect(document.activeElement).not.toBe(view.canvas)
})

test('an inactive tab releases keys even when the canvas never had focus', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  view.driver.key.mockClear()

  await view.setActive(false)

  expect(keys(view.driver)).toEqual([[false, 0x61]])
})

test('a session that ends takes focus off the hidden canvas', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(document.activeElement).not.toBe(view.canvas)
})

test('reconnecting releases held keys on the old session before closing it', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.driver.key.mock.calls).toEqual([
    ['s1', true, 0x61, 'KeyA'],
    ['s1', false, 0x61, 'KeyA'],
  ])
  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(view.driver.key.mock.invocationCallOrder[1]).toBeLessThan(
    view.driver.close.mock.invocationCallOrder[0],
  )
  expect(view.driver.open).toHaveBeenCalledTimes(2)

  await view.open(session('s2'))
  fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'b' })
  expect(view.driver.key.mock.calls[2]).toEqual(['s2', true, 0x62, 'KeyB'])
})

test('a closing session releases held keys and takes no more input', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  await view.emit({ kind: 'closed', reason: 'bye' })

  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
  ])
  expect(fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'b' })).toBe(true)
  fireEvent.pointerDown(view.canvas, { clientX: 1, clientY: 1, buttons: 1, pointerId: 1 })
  fireEvent.wheel(view.canvas, { deltaY: 100 })
  expect(view.driver.key).toHaveBeenCalledTimes(2)
  expect(view.driver.pointer).not.toHaveBeenCalled()
})

test('unmount releases held keys and a held-back Control never fires afterwards', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS)
  const view = await connect()
  vi.useFakeTimers()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.keyDown(view.canvas, { code: 'ControlLeft', key: 'Control' })
  expect(keys(view.driver)).toEqual([[true, 0x61]])

  view.unmount()
  vi.advanceTimersByTime(1000)

  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
  ])
  expect(view.driver.close.mock.calls).toEqual([['s1']])
})

test('pointer coordinates are scaled from the displayed size to framebuffer pixels', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 100, 50)
  fireEvent.pointerDown(view.canvas, { clientX: 50, clientY: 25, buttons: 1, pointerId: 1 })

  expect(view.driver.pointer.mock.calls).toEqual([['s1', 1, 100, 50]])
})

test('pointer coordinates are relative to where the canvas is displayed', async () => {
  const view = await connect()
  displayAt(view.canvas, 10, 20, 100, 50)
  fireEvent.pointerDown(view.canvas, { clientX: 60, clientY: 45, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 10, clientY: 20, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 109.9, clientY: 69.9, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 20.3, clientY: 30.3, buttons: 0, pointerId: 1 })

  expect(points(view.driver)).toEqual([
    [1, 100, 50],
    [0, 0, 0],
    [1, 199, 99],
    [0, 20, 20],
  ])
})

test.each([
  [-5, -5, 0, 0],
  [500, 500, 199, 99],
  [100, 50, 199, 99],
  [-5, 25, 0, 50],
  [50, 500, 100, 99],
])(
  'a pointer at (%d, %d) outside the canvas clamps to (%d, %d)',
  async (clientX, clientY, x, y) => {
    const view = await connect()
    displayAt(view.canvas, 0, 0, 100, 50)
    fireEvent.pointerDown(view.canvas, { clientX, clientY, buttons: 1, pointerId: 1 })

    expect(points(view.driver)).toEqual([[1, x, y]])
  },
)

test('a box of another shape than the picture is mapped through its letterbox', async () => {
  const view = await connect()
  await view.emit({ kind: 'resize', w: 100, h: 100 })
  displayAt(view.canvas, 10, 0, 200, 50)
  fireEvent.pointerDown(view.canvas, { clientX: 110, clientY: 25, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 20, clientY: 25, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 134.9, clientY: 49.9, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 200, clientY: 0, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 85, clientY: 0, buttons: 1, pointerId: 1 })

  expect(points(view.driver)).toEqual([
    [1, 50, 50],
    [0, 0, 50],
    [1, 99, 99],
    [0, 99, 0],
    [1, 0, 0],
  ])
})

test('a taller box than the picture is mapped through its letterbox too', async () => {
  const view = await connect()
  await view.emit({ kind: 'resize', w: 100, h: 100 })
  displayAt(view.canvas, 0, 10, 50, 200)
  fireEvent.pointerDown(view.canvas, { clientX: 25, clientY: 110, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 25, clientY: 20, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 49.9, clientY: 134.9, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 0, clientY: 200, buttons: 0, pointerId: 1 })

  expect(points(view.driver)).toEqual([
    [1, 50, 50],
    [0, 50, 0],
    [1, 99, 99],
    [0, 0, 99],
  ])
})

test('a box larger than the picture is mapped back to framebuffer pixels', async () => {
  const view = await connect()
  await view.emit({ kind: 'resize', w: 640, h: 400 })
  displayAt(view.canvas, 0, 0, 1280, 1000)
  fireEvent.pointerDown(view.canvas, { clientX: 640, clientY: 500, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 640, clientY: 50, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 1279.9, clientY: 899.9, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 0, clientY: 950, buttons: 0, pointerId: 1 })
  fireEvent.pointerDown(view.canvas, { clientX: 3, clientY: 100, buttons: 1, pointerId: 1 })

  expect(points(view.driver)).toEqual([
    [1, 320, 200],
    [0, 320, 0],
    [1, 639, 399],
    [0, 0, 399],
    [1, 1, 0],
  ])
})

test('the canvas fills its area in both directions and keeps the shape of the picture', async () => {
  const view = await connect()

  expect(view.canvas).toHaveClass('h-full', 'w-full', 'object-contain')
  expect(view.canvas).not.toHaveClass('max-h-full')
  expect(view.canvas).not.toHaveClass('max-w-full')
})

test.each([
  ['a display rectangle without width', 200, 100, 0, 50],
  ['a display rectangle without height', 200, 100, 100, 0],
  ['a framebuffer without width', 0, 100, 100, 50],
  ['a framebuffer without height', 200, 0, 100, 50],
])('nothing is sent for %s', async (_, w, h, shownW, shownH) => {
  const view = await connect()
  await view.emit({ kind: 'resize', w, h })
  displayAt(view.canvas, 0, 0, shownW, shownH)
  fireEvent.pointerDown(view.canvas, { clientX: 5, clientY: 5, buttons: 1, pointerId: 1 })
  fireEvent.pointerMove(view.canvas, { clientX: 6, clientY: 6, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 6, clientY: 6, buttons: 0, pointerId: 1 })
  fireEvent.wheel(view.canvas, { deltaY: 100, clientX: 5, clientY: 5 })
  runFrames()

  expect(view.driver.pointer).not.toHaveBeenCalled()
})

test.each(['pointerCancel', 'lostPointerCapture'] as const)(
  '%s releases the held buttons at the last position',
  async (event) => {
    const view = await connect()
    displayAt(view.canvas, 0, 0, 200, 100)
    fireEvent.pointerDown(view.canvas, { clientX: 20, clientY: 30, buttons: 1 | 2, pointerId: 1 })
    fireEvent.pointerMove(view.canvas, { clientX: 40, clientY: 50, buttons: 1 | 2, pointerId: 1 })
    fireEvent[event](view.canvas, { pointerId: 1 })
    runFrames()

    expect(points(view.driver)).toEqual([
      [5, 20, 30],
      [0, 40, 50],
    ])

    fireEvent[event](view.canvas, { pointerId: 1 })
    expect(view.driver.pointer).toHaveBeenCalledTimes(2)
  },
)

test('blur releases the held buttons', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 20, clientY: 30, buttons: 1, pointerId: 1 })
  act(() => view.canvas.blur())

  expect(points(view.driver)).toEqual([
    [1, 20, 30],
    [0, 20, 30],
  ])
})

test('the tab becoming inactive releases the held buttons once', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 20, clientY: 30, buttons: 4, pointerId: 1 })
  await view.setActive(false)

  expect(points(view.driver)).toEqual([
    [2, 20, 30],
    [0, 20, 30],
  ])
})

test('an inactive tab releases buttons even when the canvas never had focus', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerMove(view.canvas, { clientX: 20, clientY: 30, buttons: 1, pointerId: 1 })
  await view.setActive(false)

  expect(points(view.driver)).toEqual([
    [1, 20, 30],
    [0, 20, 30],
  ])
})

test('with no button held, cancel, lost capture, blur and inactive send nothing', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 20, clientY: 30, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 20, clientY: 30, buttons: 0, pointerId: 1 })
  fireEvent.pointerCancel(view.canvas, { pointerId: 1 })
  fireEvent.lostPointerCapture(view.canvas, { pointerId: 1 })
  act(() => view.canvas.blur())
  await view.setActive(false)

  expect(view.driver.pointer).toHaveBeenCalledTimes(2)
})

test('pointer coordinates follow a resize', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 100, 50)
  await view.emit({ kind: 'resize', w: 400, h: 200 })
  fireEvent.pointerDown(view.canvas, { clientX: 50, clientY: 25, buttons: 1, pointerId: 1 })

  expect(points(view.driver)).toEqual([[1, 200, 100]])
})

test('pointerdown captures the pointer and focuses the canvas', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 1, clientY: 1, buttons: 1, pointerId: 7 })

  expect(setPointerCapture).toHaveBeenCalledWith(7)
  expect(setPointerCapture.mock.contexts[0]).toBe(view.canvas)
  expect(document.activeElement).toBe(view.canvas)
})

test('buttons are sent as the VNC mask', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 1, clientY: 1, buttons: 2, pointerId: 1 })
  fireEvent.pointerMove(view.canvas, { clientX: 1, clientY: 1, buttons: 2 | 4, pointerId: 1 })
  fireEvent.pointerMove(view.canvas, { clientX: 1, clientY: 1, buttons: 4, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 1, clientY: 1, buttons: 0, pointerId: 1 })

  expect(points(view.driver)).toEqual([
    [4, 1, 1],
    [6, 1, 1],
    [2, 1, 1],
    [0, 1, 1],
  ])
  expect(frames).toHaveLength(0)
})

test('moves are sent once per animation frame with the latest position', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerMove(view.canvas, { clientX: 10, clientY: 10, buttons: 0 })
  fireEvent.pointerMove(view.canvas, { clientX: 20, clientY: 20, buttons: 0 })
  fireEvent.pointerMove(view.canvas, { clientX: 30, clientY: 40, buttons: 0 })
  expect(view.driver.pointer).not.toHaveBeenCalled()
  expect(frames).toHaveLength(1)

  runFrames()
  expect(points(view.driver)).toEqual([[0, 30, 40]])

  fireEvent.pointerMove(view.canvas, { clientX: 50, clientY: 60, buttons: 0 })
  runFrames()
  expect(points(view.driver)).toEqual([
    [0, 30, 40],
    [0, 50, 60],
  ])
})

test('a button change is sent at once and replaces a waiting move', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerMove(view.canvas, { clientX: 10, clientY: 10, buttons: 0 })
  fireEvent.pointerDown(view.canvas, { clientX: 20, clientY: 20, buttons: 1, pointerId: 1 })
  expect(points(view.driver)).toEqual([[1, 20, 20]])

  runFrames()
  expect(points(view.driver)).toEqual([[1, 20, 20]])

  fireEvent.pointerMove(view.canvas, { clientX: 30, clientY: 30, buttons: 1, pointerId: 1 })
  fireEvent.pointerUp(view.canvas, { clientX: 40, clientY: 40, buttons: 0, pointerId: 1 })
  runFrames()
  expect(points(view.driver)).toEqual([
    [1, 20, 20],
    [0, 40, 40],
  ])
})

test('a waiting move is dropped when the session ends', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerMove(view.canvas, { clientX: 10, clientY: 10, buttons: 0 })
  await view.emit({ kind: 'closed', reason: 'bye' })
  runFrames()

  expect(view.driver.pointer).not.toHaveBeenCalled()
})

test('a click right after Control arrives with Control on Windows', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS)
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.keyDown(view.canvas, { code: 'ControlLeft', key: 'Control' })
  expect(view.driver.key).not.toHaveBeenCalled()

  fireEvent.pointerDown(view.canvas, { clientX: 5, clientY: 5, buttons: 1, pointerId: 1 })

  expect(view.driver.key.mock.calls).toEqual([['s1', true, 0xffe3, 'ControlLeft']])
  expect(view.driver.key.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.pointer.mock.invocationCallOrder[0],
  )
})

test('a move does not send a held-back Control', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS)
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.keyDown(view.canvas, { code: 'ControlLeft', key: 'Control' })
  fireEvent.pointerMove(view.canvas, { clientX: 5, clientY: 5, buttons: 0 })
  runFrames()

  expect(view.driver.pointer).toHaveBeenCalledTimes(1)
  expect(view.driver.key).not.toHaveBeenCalled()
})

test('the context menu is suppressed on the canvas', async () => {
  const view = await connect()
  expect(fireEvent.contextMenu(view.canvas)).toBe(false)
})

test.each([
  [-50, 8],
  [50, 16],
])('a wheel step of %d presses and releases mask %d', async (deltaY, mask) => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 100, 50)
  expect(fireEvent.wheel(view.canvas, { deltaY, clientX: 50, clientY: 25 })).toBe(false)

  expect(points(view.driver)).toEqual([
    [mask, 100, 50],
    [0, 100, 50],
  ])
})

test('wheel steps keep the held buttons and add up small deltas', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.pointerDown(view.canvas, { clientX: 5, clientY: 5, buttons: 1, pointerId: 1 })
  view.driver.pointer.mockClear()

  fireEvent.wheel(view.canvas, { deltaY: 30, clientX: 5, clientY: 5 })
  expect(view.driver.pointer).not.toHaveBeenCalled()

  fireEvent.wheel(view.canvas, { deltaY: 70, clientX: 6, clientY: 7 })
  expect(points(view.driver)).toEqual([
    [17, 6, 7],
    [1, 6, 7],
    [17, 6, 7],
    [1, 6, 7],
  ])
})

test('a wheel line counts as a step sideways too', async () => {
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.wheel(view.canvas, { deltaX: 3, deltaMode: 1, clientX: 5, clientY: 5 })

  expect(points(view.driver)).toEqual([
    [64, 5, 5],
    [0, 5, 5],
  ])
})

test('a wheel step right after Control arrives with Control on Windows', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS)
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  fireEvent.keyDown(view.canvas, { code: 'ControlLeft', key: 'Control' })
  fireEvent.wheel(view.canvas, { deltaY: -50, clientX: 5, clientY: 5 })

  expect(view.driver.key.mock.calls).toEqual([['s1', true, 0xffe3, 'ControlLeft']])
  expect(view.driver.key.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.pointer.mock.invocationCallOrder[0],
  )
})

test('with clipboard sync off nothing is read or written', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  await view.setActive(false)
  await view.setActive(true)
  await view.emit({ kind: 'clipboard', text: 'remote' })

  expect(clipboard.readText).not.toHaveBeenCalled()
  expect(clipboard.writeText).not.toHaveBeenCalled()
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('with clipboard sync on, focus sends the local text once', async () => {
  setClipboardSync(true)
  const view = await connect()
  await act(async () => view.canvas.focus())

  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'local']])

  await act(async () => view.canvas.blur())
  await act(async () => view.canvas.focus())
  expect(clipboard.readText).toHaveBeenCalledTimes(2)
  expect(view.driver.clipboard).toHaveBeenCalledTimes(1)

  clipboard.readText.mockResolvedValue('newer')
  await act(async () => view.canvas.blur())
  await act(async () => view.canvas.focus())
  expect(view.driver.clipboard.mock.calls).toEqual([
    ['s1', 'local'],
    ['s1', 'newer'],
  ])
})

test('with clipboard sync on, the tab becoming active sends the local text', async () => {
  setClipboardSync(true)
  const view = await connect(false)
  expect(clipboard.readText).not.toHaveBeenCalled()

  await view.setActive(true)

  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'local']])
})

test('with clipboard sync on, remote text is written locally and not sent back', async () => {
  setClipboardSync(true)
  const view = await connect()
  await view.emit({ kind: 'clipboard', text: 'remote' })
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])

  clipboard.readText.mockResolvedValue('remote')
  await act(async () => view.canvas.focus())
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('an empty local clipboard is not sent to a fresh session', async () => {
  setClipboardSync(true)
  clipboard.readText.mockResolvedValue('')
  const view = await connect()
  await act(async () => view.canvas.focus())

  expect(clipboard.readText).toHaveBeenCalledTimes(1)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('a refused clipboard read shows no error', async () => {
  setClipboardSync(true)
  clipboard.readText.mockRejectedValue(new Error('not focused'))
  const view = await connect()

  expect(await unhandledRejections(() => act(async () => view.canvas.focus()))).toBe(0)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
})

test('a refused clipboard write shows no error', async () => {
  setClipboardSync(true)
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { ...clipboard, writeText: () => Promise.reject(new Error('not focused')) },
  })
  const view = await connect()

  expect(await unhandledRejections(() => view.emit({ kind: 'clipboard', text: 'remote' }))).toBe(0)
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
})

test('refused remote text gives way to text copied locally since', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  localText = 'copied here'

  await act(async () => view.canvas.focus())
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'copied here']])
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
  expect(localText).toBe('copied here')

  await focusAgain(view.canvas)
  expect(view.driver.clipboard).toHaveBeenCalledTimes(1)
  expect(clipboard.writeText).toHaveBeenCalledTimes(1)
  expect(localText).toBe('copied here')
})

test('refused remote text is written again on the next focus when nothing was copied locally', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  expect(localText).toBe('local')

  await act(async () => view.canvas.focus())
  expect(clipboard.writeText.mock.calls).toEqual([['remote'], ['remote']])
  expect(localText).toBe('remote')
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  await focusAgain(view.canvas)
  expect(clipboard.writeText).toHaveBeenCalledTimes(2)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('text copied locally wins after the remote text was refused on several focuses', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValue(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  await act(async () => view.canvas.focus())
  await focusAgain(view.canvas)
  expect(clipboard.writeText.mock.calls).toEqual([['remote'], ['remote'], ['remote']])
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  localText = 'copied here'
  await focusAgain(view.canvas)
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'copied here']])
  expect(clipboard.writeText).toHaveBeenCalledTimes(3)

  await focusAgain(view.canvas)
  expect(view.driver.clipboard).toHaveBeenCalledTimes(1)
  expect(clipboard.writeText).toHaveBeenCalledTimes(3)
})

test('remote text refused before clipboard sync was switched off is dropped', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })

  setClipboardSync(false)
  await act(async () => view.canvas.focus())
  expect(clipboard.readText).not.toHaveBeenCalled()

  setClipboardSync(true)
  await focusAgain(view.canvas)
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  localText = 'copied here'
  await focusAgain(view.canvas)
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'copied here']])
})

test('remote text that arrives with clipboard sync off clears what was pending', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  setClipboardSync(false)
  await view.emit({ kind: 'clipboard', text: 'ignored' })
  setClipboardSync(true)

  await act(async () => view.canvas.focus())
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('local text read before newer remote text arrived is not sent', async () => {
  const view = await connectSynced()
  let finish: (text: string) => void = () => {}
  clipboard.readText.mockReturnValueOnce(
    new Promise<string>((resolve) => {
      finish = resolve
    }),
  )
  await act(async () => view.canvas.focus())
  await view.emit({ kind: 'clipboard', text: 'remote' })
  await act(async () => finish('stale'))

  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(localText).toBe('remote')
})

test('two remote texts in a row are not sent back and the second one ends up local', async () => {
  const view = await connectSynced()
  heldWrites = []
  await view.emit({ kind: 'clipboard', text: 'first' })
  await view.emit({ kind: 'clipboard', text: 'second' })
  await act(async () => view.canvas.focus())
  await landWrites(3)

  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(localText).toBe('second')

  heldWrites = null
  await focusAgain(view.canvas)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(localText).toBe('second')
})

test('an older remote text that lands first is not taken for a local copy', async () => {
  const view = await connectSynced()
  heldWrites = []
  await view.emit({ kind: 'clipboard', text: 'first' })
  await view.emit({ kind: 'clipboard', text: 'second' })
  await landWrites(1)
  expect(localText).toBe('first')

  await act(async () => view.canvas.focus())
  await landWrites(2)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(localText).toBe('second')
})

test('an older remote text that lands late does not clear newer refused text', async () => {
  const view = await connectSynced()
  heldWrites = []
  await view.emit({ kind: 'clipboard', text: 'first' })
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'second' })
  await landWrites(1)
  heldWrites = null
  expect(localText).toBe('first')

  await act(async () => view.canvas.focus())
  expect(localText).toBe('second')
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('local text read before clipboard sync was switched off is not sent', async () => {
  const view = await connectSynced()
  let finish: (text: string) => void = () => {}
  clipboard.readText.mockReturnValueOnce(
    new Promise<string>((resolve) => {
      finish = resolve
    }),
  )
  await act(async () => view.canvas.focus())
  setClipboardSync(false)
  await act(async () => finish('copied here'))
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  setClipboardSync(true)
  localText = 'copied here'
  await focusAgain(view.canvas)
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'copied here']])
})

test('an empty local clipboard is not sent', async () => {
  const view = await connectSynced()
  localText = ''
  await act(async () => view.canvas.focus())
  expect(clipboard.readText).toHaveBeenCalledTimes(1)
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  localText = 'local'
  await focusAgain(view.canvas)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('an empty local clipboard does not drop refused remote text', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  localText = ''

  await act(async () => view.canvas.focus())
  expect(clipboard.writeText.mock.calls).toEqual([['remote'], ['remote']])
  expect(localText).toBe('remote')
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('local text equal to the refused remote text is not sent back', async () => {
  const view = await connectSynced()
  clipboard.writeText.mockRejectedValueOnce(new Error('not focused'))
  await view.emit({ kind: 'clipboard', text: 'remote' })
  localText = 'remote'

  await act(async () => view.canvas.focus())
  await focusAgain(view.canvas)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
})

test('local text read for a session that ended meanwhile is not sent', async () => {
  setClipboardSync(true)
  let finish: (text: string) => void = () => {}
  clipboard.readText.mockReturnValue(
    new Promise<string>((resolve) => {
      finish = resolve
    }),
  )
  const view = await connect()
  await act(async () => view.canvas.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })
  await act(async () => finish('late'))

  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('remote text for a hidden tab is held and written once the tab is shown', async () => {
  const view = await connectSynced()
  await view.setActive(false)
  await view.emit({ kind: 'clipboard', text: 'remote' })
  expect(clipboard.writeText).not.toHaveBeenCalled()
  expect(localText).toBe('local')

  await view.setActive(true)
  expect(clipboard.writeText.mock.calls).toEqual([['remote']])
  expect(localText).toBe('remote')
  expect(view.driver.clipboard).not.toHaveBeenCalled()

  await view.setActive(false)
  await view.setActive(true)
  expect(clipboard.writeText).toHaveBeenCalledTimes(1)
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('text copied locally while the tab was hidden wins over the remote text held for it', async () => {
  const view = await connectSynced()
  await view.setActive(false)
  await view.emit({ kind: 'clipboard', text: 'remote' })
  localText = 'copied here'

  await view.setActive(true)
  expect(clipboard.writeText).not.toHaveBeenCalled()
  expect(localText).toBe('copied here')
  expect(view.driver.clipboard.mock.calls).toEqual([['s1', 'copied here']])

  await view.setActive(false)
  await view.setActive(true)
  expect(clipboard.writeText).not.toHaveBeenCalled()
  expect(view.driver.clipboard).toHaveBeenCalledTimes(1)
})

test('the last of several remote texts for a hidden tab is the one written', async () => {
  const view = await connectSynced()
  await view.setActive(false)
  await view.emit({ kind: 'clipboard', text: 'first' }, { kind: 'clipboard', text: 'second' })
  expect(clipboard.writeText).not.toHaveBeenCalled()

  await view.setActive(true)
  expect(clipboard.writeText.mock.calls).toEqual([['second']])
  expect(view.driver.clipboard).not.toHaveBeenCalled()
})

test('a cursor message becomes a data-URL cursor with its hotspot', async () => {
  const view = await connect()
  const rgba = pixels(16, 24)
  await view.emit({ kind: 'cursor', hotX: 3, hotY: 5, w: 16, h: 24, rgba })

  expect(view.canvas.style.cursor).toBe('url(data:image/png;base64,AA) 3 5, default')
  const [image, x, y] = ctx.putImageData.mock.calls[0]
  expect([image.width, image.height, x, y]).toEqual([16, 24, 0, 0])
  expect(image.data).toBe(rgba)
  const offscreen = vi.mocked(HTMLCanvasElement.prototype.toDataURL).mock
    .contexts[0] as HTMLCanvasElement
  expect(offscreen).not.toBe(view.canvas)
  expect([offscreen.width, offscreen.height]).toEqual([16, 24])
  expect([view.canvas.width, view.canvas.height]).toEqual([200, 100])
})

test('an empty cursor hides the pointer', async () => {
  const view = await connect()
  await view.emit({ kind: 'cursor', hotX: 0, hotY: 0, w: 0, h: 0, rgba: pixels(0, 0) })

  expect(view.canvas.style.cursor).toBe('none')
  expect(ctx.putImageData).not.toHaveBeenCalled()
})

test.each([
  [128, 128, 'url(data:image/png;base64,AA) 1 2, default'],
  [129, 16, 'default'],
  [16, 129, 'default'],
])('a %dx%d cursor sets %s', async (w, h, cursor) => {
  const view = await connect()
  await view.emit({ kind: 'cursor', hotX: 1, hotY: 2, w, h, rgba: pixels(w, h) })

  expect(view.canvas.style.cursor).toBe(cursor)
})

test('a default cursor message drops the remote cursor image', async () => {
  const view = await connect()
  await view.emit({ kind: 'cursor', hotX: 3, hotY: 5, w: 16, h: 24, rgba: pixels(16, 24) })
  expect(view.canvas.style.cursor).not.toBe('default')
  await view.emit({ kind: 'cursorDefault' })

  expect(view.canvas.style.cursor).toBe('default')
})

test('the view opens the session with an unmeasured viewport', async () => {
  const view = await connect()

  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 0,
      height: 0,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('a new session starts with the default cursor', async () => {
  const view = await connect()
  await view.emit({ kind: 'cursor', hotX: 0, hotY: 0, w: 0, h: 0, rgba: pixels(0, 0) })
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  await view.open(session('s2'))

  expect(view.canvas.style.cursor).toBe('')
})

test('Ctrl+Alt+Del sends the three keys down and up in reverse', async () => {
  const view = await connect()
  fireEvent.click(screen.getByRole('button', { name: 'Ctrl+Alt+Del' }))

  expect(view.driver.key.mock.calls).toEqual([
    ['s1', true, 0xffe3, 'ControlLeft'],
    ['s1', true, 0xffe9, 'AltLeft'],
    ['s1', true, 0xffff, 'Delete'],
    ['s1', false, 0xffff, 'Delete'],
    ['s1', false, 0xffe9, 'AltLeft'],
    ['s1', false, 0xffe3, 'ControlLeft'],
  ])
})

test('Ctrl+Alt+Del gives the keyboard back to the canvas', async () => {
  const user = userEvent.setup()
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  await user.click(view.canvas)
  await user.click(screen.getByRole('button', { name: 'Ctrl+Alt+Del' }))
  expect(view.canvas).toHaveFocus()
  expect(view.driver.key).toHaveBeenCalledTimes(6)
  view.driver.key.mockClear()

  await user.keyboard('a{Enter}')

  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
    [true, 0xff0d],
    [false, 0xff0d],
  ])
})

test('Fullscreen gives the keyboard back to the canvas', async () => {
  const user = userEvent.setup()
  const view = await connect()
  displayAt(view.canvas, 0, 0, 200, 100)
  await user.click(view.canvas)
  await user.click(screen.getByRole('button', { name: 'Fullscreen' }))
  expect(view.canvas).toHaveFocus()

  await user.keyboard('a{Enter}')

  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
    [true, 0xff0d],
    [false, 0xff0d],
  ])
  expect(requestFullscreen).toHaveBeenCalledTimes(1)

  setFullscreen(view.container.firstElementChild)
  await user.click(screen.getByRole('button', { name: 'Exit fullscreen' }))
  expect(view.canvas).toHaveFocus()
  expect(exitFullscreen).toHaveBeenCalledTimes(1)
})

test('Disconnect closes the session and shows Disconnected.', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }))

  expect(screen.getByText('Disconnected.')).toBeInTheDocument()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
  ])
  expect(view.driver.key.mock.invocationCallOrder[1]).toBeLessThan(
    view.driver.close.mock.invocationCallOrder[0],
  )

  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' }, { kind: 'closed', reason: 'late' })
  expect(ctx.putImageData).not.toHaveBeenCalled()
  expect(view.driver.ack).not.toHaveBeenCalled()
  expect(screen.getByText('Disconnected.')).toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  view.unmount()
  expect(view.driver.close).toHaveBeenCalledTimes(1)
})

test('Fullscreen puts the whole view in fullscreen and the button follows', async () => {
  const view = await connect()
  const container = view.container.firstElementChild
  fireEvent.click(screen.getByRole('button', { name: 'Fullscreen' }))

  expect(requestFullscreen).toHaveBeenCalledTimes(1)
  expect(requestFullscreen.mock.contexts[0]).toBe(container)
  expect(screen.getByRole('button', { name: 'Fullscreen' })).toHaveAttribute(
    'aria-pressed',
    'false',
  )

  setFullscreen(container)
  expect(screen.queryByRole('button', { name: 'Fullscreen' })).toBeNull()
  expect(screen.getByRole('button', { name: 'Exit fullscreen' })).toHaveAttribute(
    'aria-pressed',
    'true',
  )
  expect(exitFullscreen).not.toHaveBeenCalled()

  fireEvent.click(screen.getByRole('button', { name: 'Exit fullscreen' }))
  expect(exitFullscreen).toHaveBeenCalledTimes(1)
  expect(requestFullscreen).toHaveBeenCalledTimes(1)

  setFullscreen(null)
  expect(screen.getByRole('button', { name: 'Fullscreen' })).toHaveAttribute(
    'aria-pressed',
    'false',
  )
})

test('a refused fullscreen request or exit changes nothing', async () => {
  const view = await connect()
  const container = view.container.firstElementChild
  // Plain functions: a mock handles the promises it returns, which would hide a missing catch.
  Element.prototype.requestFullscreen = () => Promise.reject(new TypeError('refused'))
  document.exitFullscreen = () => Promise.reject(new TypeError('refused'))

  expect(
    await unhandledRejections(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Fullscreen' }))
    }),
  ).toBe(0)
  expect(screen.getByRole('button', { name: 'Fullscreen' })).toBeInTheDocument()

  setFullscreen(container)
  expect(
    await unhandledRejections(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Exit fullscreen' }))
    }),
  ).toBe(0)
  expect(screen.getByRole('button', { name: 'Exit fullscreen' })).toBeInTheDocument()

  expect(await unhandledRejections(() => view.emit({ kind: 'closed', reason: 'bye' }))).toBe(0)
  expect(screen.getByText('bye')).toBeInTheDocument()
})

test('fullscreen ends when the session closes', async () => {
  const view = await connect()
  setFullscreen(view.container.firstElementChild)
  expect(exitFullscreen).not.toHaveBeenCalled()

  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(exitFullscreen).toHaveBeenCalledTimes(1)
})

test('fullscreen ends when the tab becomes inactive', async () => {
  const view = await connect()
  setFullscreen(view.container.firstElementChild)
  await view.setActive(false)

  expect(exitFullscreen).toHaveBeenCalledTimes(1)
})

test('fullscreen ends when a reconnect starts', async () => {
  const view = await connect()
  setFullscreen(view.container.firstElementChild)
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(exitFullscreen).toHaveBeenCalledTimes(1)
})

test.each([
  [
    'a certificate prompt',
    { kind: 'hostKeyUnknown', message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' } },
  ],
  ['a connect error', { kind: 'internal', message: 'vnc: wrong password' }],
])('fullscreen does not survive %s', async (_, error) => {
  const view = start()
  await view.fail(error)
  setFullscreen(view.container.firstElementChild)

  expect(exitFullscreen).toHaveBeenCalledTimes(1)
})

test('fullscreen owned by another element is left alone', async () => {
  const view = await connect()
  setFullscreen(document.body)
  expect(screen.getByRole('button', { name: 'Fullscreen' })).toHaveAttribute(
    'aria-pressed',
    'false',
  )

  await view.setActive(false)
  await view.setActive(true)
  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(exitFullscreen).not.toHaveBeenCalled()
})

test('a session that ends with focus in the view moves it to Reconnect', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(document.body).toHaveFocus()
  await view.fail({ kind: 'internal', message: 'timed out' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  await view.open(session('s2'))
  act(() => screen.getByRole('button', { name: 'Disconnect' }).focus())
  fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }))
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('focus that moved within the view still counts when the session ends', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  act(() => screen.getByRole('button', { name: 'Ctrl+Alt+Del' }).focus())
  await view.emit({ kind: 'closed', reason: 'bye' })

  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('a session that ends while focus is elsewhere in the window leaves it there', async () => {
  const elsewhere = document.createElement('button')
  document.body.appendChild(elsewhere)
  const view = await connect()
  act(() => view.canvas.focus())
  act(() => elsewhere.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })

  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(elsewhere).toHaveFocus()
  elsewhere.remove()
})

test('a session that ends with nothing focused moves focus to Reconnect', async () => {
  const view = await connect()
  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(document.body).toHaveFocus()
  await view.fail({ kind: 'internal', message: 'timed out' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('a session that ends in a hidden tab takes focus only once the tab is shown', async () => {
  const view = await connect()
  act(() => view.canvas.focus())
  await view.setActive(false)
  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(document.body).toHaveFocus()

  await view.setActive(true)
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('focus that left the canvas for nothing and then went elsewhere is not taken', async () => {
  const elsewhere = outsideButton()
  const view = await connect()
  act(() => view.canvas.focus())
  act(() => view.canvas.blur())
  act(() => elsewhere.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })

  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(elsewhere).toHaveFocus()
})

test('focus that went elsewhere after the tab was hidden and shown is not taken', async () => {
  const elsewhere = outsideButton()
  const view = await connect()
  act(() => view.canvas.focus())
  await view.setActive(false)
  await view.setActive(true)
  act(() => elsewhere.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })

  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(elsewhere).toHaveFocus()
})

test('a tab shown again with an ended session does not take focus from elsewhere', async () => {
  const elsewhere = outsideButton()
  const view = await connect()
  act(() => view.canvas.focus())
  await view.setActive(false)
  await view.emit({ kind: 'closed', reason: 'bye' })
  act(() => elsewhere.focus())
  await view.setActive(true)

  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(elsewhere).toHaveFocus()
})

test('focus that left Reconnect for nothing and went elsewhere stays there when the tab is shown again', async () => {
  const elsewhere = outsideButton()
  const view = await connect()
  act(() => view.canvas.focus())
  await view.emit({ kind: 'closed', reason: 'bye' })
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()

  act(() => screen.getByRole('button', { name: 'Reconnect' }).blur())
  act(() => elsewhere.focus())
  await view.setActive(false)
  await view.setActive(true)
  expect(elsewhere).toHaveFocus()
})

test('focus left inside the view goes to Reconnect when the tab is shown again', async () => {
  const view = await connect()
  await view.emit({ kind: 'closed', reason: 'bye' })
  act(() => screen.getByRole('button', { name: 'Close tab' }).focus())
  await view.setActive(false)
  expect(screen.getByRole('button', { name: 'Close tab' })).toHaveFocus()

  await view.setActive(true)
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('a certificate that cannot be stored moves focus to Reconnect', async () => {
  vi.mocked(trustHostKey).mockRejectedValue(new Error('io: disk full'))
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })

  expect(screen.getByText('io: disk full')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('a rejected certificate moves focus to Reconnect', async () => {
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Reject' }))
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })

  expect(screen.getByRole('button', { name: 'Reconnect' })).toHaveFocus()
})

test('trusting a certificate connects with the driver the view has by then', async () => {
  let trusted: () => void = () => {}
  vi.mocked(trustHostKey).mockReturnValue(
    new Promise<void>((resolve) => {
      trusted = resolve
    }),
  )
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })
  const next = fakeDriver()
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  expect(next.driver.open).toHaveBeenCalledTimes(1)

  await act(async () => trusted())

  expect(view.driver.open).toHaveBeenCalledTimes(1)
  expect(next.driver.open).toHaveBeenCalledTimes(2)
})

test('a certificate that could not be stored is not reported once a newer attempt runs', async () => {
  let refuse: (error: Error) => void = () => {}
  vi.mocked(trustHostKey).mockReturnValue(
    new Promise<void>((_, reject) => {
      refuse = reject
    }),
  )
  const view = start()
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'vnc/h', port: 5900, fingerprint: 'SHA256:ab' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })
  const next = fakeDriver()
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  await act(async () => refuse(new Error('io: disk full')))

  expect(screen.queryByText('io: disk full')).toBeNull()
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
  expect(next.driver.open).toHaveBeenCalledTimes(1)
  expect(view.driver.open).toHaveBeenCalledTimes(1)
})

test('a failing acknowledgement does not stop later messages', async () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  const view = await connect()
  const broke = new Error('ack broke')
  Object.assign(view.driver, {
    ack: () => {
      throw broke
    },
  })
  await view.emit({ kind: 'sync' }, rect(1, 1, 1, 1))

  expect(errors.mock.calls).toEqual([[broke]])
  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  expect(screen.queryByText(PICTURE_ERROR)).toBeNull()
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
  expect(view.driver.close).not.toHaveBeenCalled()
})

test('a failing clipboard write does not stop later messages', async () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  setClipboardSync(true)
  const broke = new Error('no clipboard')
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: () => {
        throw broke
      },
    },
  })
  const view = await connect()
  await view.emit({ kind: 'clipboard', text: 'remote' }, rect(1, 1, 1, 1), { kind: 'sync' })

  expect(errors.mock.calls).toEqual([[broke]])
  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  expect(view.driver.ack.mock.calls).toEqual([['s1']])
  expect(screen.queryByText(PICTURE_ERROR)).toBeNull()
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()
  expect(view.driver.close).not.toHaveBeenCalled()
})

test('a queued clipboard message that throws does not stop the rest of the queue', async () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  setClipboardSync(true)
  const broke = new Error('no clipboard')
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: () => {
        throw broke
      },
    },
  })
  const view = start()
  await view.emit(
    { kind: 'clipboard', text: 'remote' },
    rect(1, 1, 1, 1),
    { kind: 'sync' },
    { kind: 'clipboard', text: 'again' },
    { kind: 'sync' },
  )
  await view.open()

  expect(errors.mock.calls).toEqual([[broke], [broke]])
  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  expect(view.driver.ack.mock.calls).toEqual([['s1'], ['s1']])
  expect(view.canvas.parentElement).not.toHaveClass('invisible')
  expect(screen.queryByText(PICTURE_ERROR)).toBeNull()
})

test('a message that cannot be painted closes the session with a reason', async () => {
  const view = await connect()
  ctx.putImageData.mockImplementation(() => {
    throw new Error('IndexSizeError')
  })
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' }, rect(1, 1, 1, 1))

  expect(screen.getByText(PICTURE_ERROR)).toBeInTheDocument()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(view.driver.ack).not.toHaveBeenCalled()
  expect(ctx.putImageData).toHaveBeenCalledTimes(1)
  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
  ])
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Close tab' })).toBeInTheDocument()

  view.unmount()
  expect(view.driver.close).toHaveBeenCalledTimes(1)
})

test('a queued message that cannot be painted closes the session as it opens', async () => {
  const view = start()
  ctx.putImageData.mockImplementation(() => {
    throw new Error('IndexSizeError')
  })
  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' })
  await view.open()

  expect(screen.getByText(PICTURE_ERROR)).toBeInTheDocument()
  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(view.driver.ack).not.toHaveBeenCalled()
})

test('a close that fails when the session is stopped is not left unhandled', async () => {
  const view = await connect()
  // A plain function: a mock handles the promises it returns, which would hide a missing catch.
  Object.assign(view.driver, { close: () => Promise.reject(new Error('ipc')) })

  expect(
    await unhandledRejections(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }))
    }),
  ).toBe(0)
  expect(screen.getByText('Disconnected.')).toBeInTheDocument()
  expect(screen.queryByText(/ipc/)).toBeNull()
})

test('a close that fails for an open that resolved after unmount is not left unhandled', async () => {
  const view = start()
  Object.assign(view.driver, { close: () => Promise.reject(new Error('ipc')) })
  view.unmount()

  expect(await unhandledRejections(() => view.open(session('late')))).toBe(0)
})

test('unmount closes the session', async () => {
  const view = await connect()
  view.unmount()

  expect(view.driver.close.mock.calls).toEqual([['s1']])
})

test('unmount after the server closed the session still closes its id', async () => {
  const view = await connect()
  await view.emit({ kind: 'closed', reason: 'bye' })
  view.unmount()

  expect(view.driver.close.mock.calls).toEqual([['s1']])
})

test('an open that resolves after unmount is closed at once', async () => {
  const view = start()
  await view.emit(rect(0, 0, 1, 1), { kind: 'sync' })
  view.unmount()
  expect(view.driver.close).not.toHaveBeenCalled()

  await view.open(session('late'))

  expect(view.driver.close.mock.calls).toEqual([['late']])
  expect(ctx.putImageData).not.toHaveBeenCalled()
  expect(view.driver.ack).not.toHaveBeenCalled()
})

test('an open that resolves after the driver changed is closed at once', async () => {
  const view = start()
  const next = fakeDriver()
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  await view.open(session('late'))

  expect(view.driver.close.mock.calls).toEqual([['late']])
  expect(next.driver.open).toHaveBeenCalledTimes(1)
  expect(next.driver.close).not.toHaveBeenCalled()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
})

test('an open that fails after the driver changed is ignored', async () => {
  const view = start()
  const next = fakeDriver()
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  await view.fail({ kind: 'internal', message: 'timed out' })

  expect(screen.queryByText('timed out')).toBeNull()
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
})

test('an open that fails after unmount is ignored', async () => {
  const view = start()
  view.unmount()
  await view.fail({ kind: 'internal', message: 'timed out' })

  expect(view.driver.close).not.toHaveBeenCalled()
})

test('unmount while connecting cancels the pending open at once', async () => {
  const view = start(true, true)
  expect(view.opens[0].signal?.aborted).toBe(false)
  view.unmount()

  expect(view.opens[0].signal?.aborted).toBe(true)
  await view.fail({ kind: 'cancelled' })
  expect(view.driver.close).not.toHaveBeenCalled()
})

test('a driver change while connecting cancels the pending open of the old driver', async () => {
  const view = start(true, true)
  const next = fakeDriver(true)
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })

  expect(view.opens[0].signal?.aborted).toBe(true)
  expect(next.opens[0].signal?.aborted).toBe(false)
})

test('Reconnect while connected closes the session and opens a new one as before', async () => {
  const view = await connect(true, true)
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(view.opens[1].signal?.aborted).toBe(false)
  await view.open(session('s2'))
  expect(view.driver.close.mock.calls).toEqual([['s1']])
})

test('under StrictMode one session is opened and the first open is cancelled', async () => {
  const fake = fakeDriver(true)
  render(
    <StrictMode>
      <RemoteDesktopView tabId="t1" driver={fake.driver} active />
    </StrictMode>,
  )

  expect(fake.opens.map((open) => open.signal?.aborted)).toEqual([true, false])
  await act(async () => fake.opens[0].reject({ kind: 'cancelled' }))
  await act(async () => fake.opens[1].resolve(session('s2')))

  expect(screen.queryByText('Connecting…')).toBeNull()
  expect(screen.queryByText('cancelled')).toBeNull()
  expect(fake.driver.close).not.toHaveBeenCalled()
  fireEvent.keyDown(screen.getByLabelText('Remote desktop'), { code: 'KeyA', key: 'a' })
  expect(fake.driver.key.mock.calls).toEqual([['s2', true, 0x61, 'KeyA']])
})

test('a new driver closes the old session and connects with the new one', async () => {
  const view = await connect()
  const next = fakeDriver()
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })

  expect(view.driver.close.mock.calls).toEqual([['s1']])
  expect(next.driver.open).toHaveBeenCalledTimes(1)
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
})

test('a re-render with the same driver does not reconnect', async () => {
  const view = await connect()
  await view.setActive(false)
  await view.setActive(true)

  expect(view.driver.open).toHaveBeenCalledTimes(1)
  expect(view.driver.close).not.toHaveBeenCalled()
})

test('the viewport passed to open is the size of the canvas area in device pixels', async () => {
  area = { width: 1200, height: 800 }
  vi.stubGlobal('devicePixelRatio', 1.5)
  const view = await connect(true, true)

  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 1800,
      height: 1200,
      scale: 150,
    },
    expect.any(AbortSignal),
  )
})

test('the viewport is rounded to whole device pixels and the scale to whole percent', async () => {
  area = { width: 801, height: 601 }
  vi.stubGlobal('devicePixelRatio', 1.25)
  const view = await connect(true, true)

  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 1001,
      height: 751,
      scale: 125,
    },
    expect.any(AbortSignal),
  )
  vi.stubGlobal('devicePixelRatio', 1.1)
  area = { width: 1001, height: 1001 }
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(view.driver.open).toHaveBeenLastCalledWith(
    expect.any(Function),
    {
      width: 1101,
      height: 1101,
      scale: 110,
    },
    expect.any(AbortSignal),
  )
})

test.each([
  [0, 0],
  [0, 600],
  [800, 0],
])('an area of %i by %i (a hidden view) is passed as zeros', async (width, height) => {
  area = { width, height }
  vi.stubGlobal('devicePixelRatio', 2)
  const view = await connect(true, true)

  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 0,
      height: 0,
      scale: 200,
    },
    expect.any(AbortSignal),
  )
})

test('the size is watched on the element that holds the canvas', async () => {
  const view = await connect(true, true)

  expect(observers).toHaveLength(1)
  expect(observers[0].observed).toEqual([view.canvas.parentElement])
})

test('a reconnect measures the area again', async () => {
  area = { width: 1000, height: 700 }
  const view = await connect(true, true)
  area = { width: 640, height: 480 }
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.driver.open).toHaveBeenLastCalledWith(
    expect.any(Function),
    {
      width: 640,
      height: 480,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('ten size changes within 300 ms give one resize with the last size', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  for (let i = 1; i <= 10; i++) {
    resizeTo(1000 + i * 10, 700 + i * 5)
    await wait(29)
  }
  expect(view.resize).not.toHaveBeenCalled()

  await wait(270)
  expect(view.resize).not.toHaveBeenCalled()
  await wait(1)
  expect(view.resize.mock.calls).toEqual([['s1', { width: 1100, height: 750, scale: 100 }]])

  await wait(1000)
  expect(view.resize).toHaveBeenCalledTimes(1)
})

test('a resize uses the device pixel ratio', async () => {
  area = { width: 1000, height: 700 }
  vi.stubGlobal('devicePixelRatio', 1.5)
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(1200, 800)
  await wait(300)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1800, height: 1200, scale: 150 }]])
})

test('a size equal to the one the session opened with is not sent', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(1000, 700)
  await wait(300)

  expect(view.resize).not.toHaveBeenCalled()
})

test('a size equal to the last one sent is not sent again, one that differs is', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(900, 600)
  await wait(300)
  resizeTo(1000, 700)
  resizeTo(900, 600)
  await wait(300)
  expect(view.resize).toHaveBeenCalledTimes(1)

  resizeTo(1000, 700)
  await wait(300)
  expect(view.resize.mock.calls).toEqual([
    ['s1', { width: 900, height: 600, scale: 100 }],
    ['s1', { width: 1000, height: 700, scale: 100 }],
  ])
})

test('a view that opened unmeasured sends its size once it is shown', async () => {
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(1000, 700)
  await wait(300)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1000, height: 700, scale: 100 }]])
})

test.each([
  [0, 0],
  [0, 600],
  [800, 0],
])('a size of %i by %i is never sent', async (width, height) => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(width, height)
  await wait(300)

  expect(view.resize).not.toHaveBeenCalled()
})

test('a size change that settles before the session is open is sent once it opens', async () => {
  useFakeTimers()
  const view = start(true, true)
  resizeTo(1000, 700)
  await wait(500)
  expect(view.resize).not.toHaveBeenCalled()

  await view.open()
  expect(view.resize.mock.calls).toEqual([['s1', { width: 1000, height: 700, scale: 100 }]])
  await wait(500)
  expect(view.resize).toHaveBeenCalledTimes(1)
})

test('a session that opens at the size of the area is not sent it again', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = start(true, true)
  await view.open()
  await wait(500)

  expect(view.resize).not.toHaveBeenCalled()
})

test('a size change that is still waiting when the session opens is sent', async () => {
  useFakeTimers()
  const view = start(true, true)
  resizeTo(1000, 700)
  await wait(200)
  await view.open()
  await wait(100)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1000, height: 700, scale: 100 }]])
})

test('a session that has ended gets no resize', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  await view.emit({ kind: 'closed', reason: 'gone' })
  resizeTo(900, 600)
  await wait(300)

  expect(view.resize).not.toHaveBeenCalled()
})

test('a size change that is waiting when the session ends is not sent', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(900, 600)
  await wait(200)
  await view.emit({ kind: 'closed', reason: 'gone' })
  await wait(200)

  expect(view.resize).not.toHaveBeenCalled()
})

test('a size change that is waiting at unmount is not sent and the observer stops', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(900, 600)
  await wait(200)
  view.unmount()
  await wait(500)

  expect(view.resize).not.toHaveBeenCalled()
  expect(observers.every((observer) => observer.disconnected)).toBe(true)
})

test('after a reconnect the size is compared with what the new session opened with', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(900, 600)
  await wait(300)
  expect(view.resize).toHaveBeenCalledTimes(1)

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  await view.open(session('s2'))
  resizeTo(900, 600)
  await wait(300)
  expect(view.resize).toHaveBeenCalledTimes(1)

  resizeTo(1000, 700)
  await wait(300)
  expect(view.resize.mock.calls.at(-1)).toEqual(['s2', { width: 1000, height: 700, scale: 100 }])
})

test('a resize that fails is not left unhandled', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  let calls = 0
  // A plain function: a mock handles the promises it returns, which would hide a missing catch.
  Object.assign(view.driver, {
    resize: () => {
      calls += 1
      return Promise.reject(new Error('gone'))
    },
  })

  expect(
    await unhandledRejections(async () => {
      resizeTo(900, 600)
      await wait(300)
      vi.useRealTimers()
    }),
  ).toBe(0)
  expect(calls).toBe(1)
})

test('a driver without resize is never asked to resize and watches nothing', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect()
  resizeTo(900, 600)
  await wait(1000)

  expect(observers).toEqual([])
  expect(view.resize).not.toHaveBeenCalled()
  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 1000,
      height: 700,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

const LOCKS = { modifierCapsLock: true, modifierNumLock: true, modifierScrollLock: false }

test('the first key sends the lock keys first, the second key does not repeat them', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', ...LOCKS })
  fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'a', ...LOCKS })
  fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'b', ...LOCKS })

  expect(view.locks.mock.calls).toEqual([['s1', { caps: true, num: true, scroll: false }]])
  expect(keys(view.driver)).toEqual([
    [true, 0x61],
    [false, 0x61],
    [true, 0x62],
  ])
  expect(view.locks.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.key.mock.invocationCallOrder[0],
  )
})

test('each lock key is read from the key event', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', modifierScrollLock: true })

  expect(view.locks.mock.calls).toEqual([['s1', { caps: false, num: false, scroll: true }]])
})

test.each([
  ['CapsLock', 'CapsLock'],
  ['NumLock', 'NumLock'],
  ['ScrollLock', 'ScrollLock'],
])('pressing %s first sends no lock keys for that press', async (code, key) => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code, key, ...LOCKS })
  fireEvent.keyUp(view.canvas, { code, key, ...LOCKS })

  expect(view.locks).not.toHaveBeenCalled()
})

test('the key after a lock key is still preceded by the lock keys, once', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'CapsLock', key: 'CapsLock', modifierCapsLock: true })
  fireEvent.keyUp(view.canvas, { code: 'CapsLock', key: 'CapsLock', modifierCapsLock: true })
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'A', modifierCapsLock: true })
  fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'B', modifierCapsLock: true })

  expect(view.locks.mock.calls).toEqual([['s1', { caps: true, num: false, scroll: false }]])
})

test('the lock keys are sent again for the first key after the canvas regains focus', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'a' })
  expect(view.locks).toHaveBeenCalledTimes(1)

  await focusAgain(view.canvas)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', modifierNumLock: true })
  fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'b', modifierNumLock: true })

  expect(view.locks.mock.calls).toEqual([
    ['s1', { caps: false, num: false, scroll: false }],
    ['s1', { caps: false, num: true, scroll: false }],
  ])
})

test('the first key of a new session sends the lock keys to that session', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  await view.open(session('s2'))
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })

  expect(view.locks.mock.calls.map(([id]) => id)).toEqual(['s1', 's2'])
})

test('a key pressed during composition does not use up the lock keys', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', isComposing: true })
  expect(view.locks).not.toHaveBeenCalled()

  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a' })
  expect(view.locks).toHaveBeenCalledTimes(1)
})

test('a key the keyboard does not send does not use up the lock keys', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: '', key: 'Dead' })
  expect(view.driver.key).not.toHaveBeenCalled()
  expect(view.locks).not.toHaveBeenCalled()

  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', ...LOCKS })
  expect(view.locks.mock.calls).toEqual([['s1', { caps: true, num: true, scroll: false }]])
  expect(view.locks.mock.invocationCallOrder[0]).toBeLessThan(
    view.driver.key.mock.invocationCallOrder[0],
  )
})

test('the lock keys read from a dropped key before the canvas lost focus are not sent', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: '', key: 'Dead', ...LOCKS })
  await focusAgain(view.canvas)
  fireEvent.keyDown(view.canvas, { code: 'CapsLock', key: 'CapsLock' })

  expect(view.locks).not.toHaveBeenCalled()
  expect(keys(view.driver)).toEqual([[true, 0xffe5]])
})

test('a dead key reaches a driver that sends scancodes by its code and is consumed', async () => {
  const view = await connect(true, true)
  expect(fireEvent.keyDown(view.canvas, { code: 'Equal', key: 'Dead' })).toBe(false)
  expect(fireEvent.keyUp(view.canvas, { code: 'Equal', key: 'Dead' })).toBe(false)
  fireEvent.keyDown(view.canvas, { code: 'KeyE', key: 'é' })

  expect(view.driver.key.mock.calls).toEqual([
    ['s1', true, 0, 'Equal'],
    ['s1', false, 0, 'Equal'],
    ['s1', true, 0xe9, 'KeyE'],
  ])
})

test('the keyup of a held key is sent even while the browser composes', async () => {
  const view = await connect(true, true)
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'Process' })
  fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'Process', isComposing: true })
  fireEvent.keyUp(view.canvas, { code: 'KeyB', key: 'Process', isComposing: true })

  expect(view.driver.key.mock.calls).toEqual([
    ['s1', true, 0, 'KeyA'],
    ['s1', false, 0, 'KeyA'],
  ])
})

test('a dead key is left to the browser for a driver that sends keysyms', async () => {
  const view = await connect()
  expect(fireEvent.keyDown(view.canvas, { code: 'Equal', key: 'Dead' })).toBe(true)
  fireEvent.keyDown(view.canvas, { code: 'KeyE', key: 'é' })

  expect(keys(view.driver)).toEqual([[true, 0xe9]])
})

test('a driver without locks sends keys as before', async () => {
  const view = await connect()
  fireEvent.keyDown(view.canvas, { code: 'KeyA', key: 'a', ...LOCKS })

  expect(keys(view.driver)).toEqual([[true, 0x61]])
  expect(view.locks).not.toHaveBeenCalled()
})

test('the area keeps its box while connecting and when the session has ended', async () => {
  const view = start()
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(view.canvas.parentElement).not.toHaveClass('hidden')
  await view.open()
  expect(view.canvas.parentElement).not.toHaveClass('invisible')
  await view.emit({ kind: 'closed', reason: 'gone' })
  expect(view.canvas.parentElement).toHaveClass('invisible')
  expect(view.canvas.parentElement).not.toHaveClass('hidden')
})

test('the first connect opens at the size of the area', async () => {
  area = { width: 1000, height: 700 }
  const view = start(true, true)

  expect(view.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 1000,
      height: 700,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('a reconnect from an ended session opens at the size of the area', async () => {
  area = { width: 1000, height: 700 }
  const view = await connect(true, true)
  await view.emit({ kind: 'closed', reason: 'gone' })
  area = { width: 640, height: 480 }
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.driver.open).toHaveBeenLastCalledWith(
    expect.any(Function),
    {
      width: 640,
      height: 480,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('a reconnect after a failed connect opens at the size of the area', async () => {
  area = { width: 1000, height: 700 }
  const view = start(true, true)
  await view.fail(new Error('refused'))
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))

  expect(view.driver.open).toHaveBeenLastCalledWith(
    expect.any(Function),
    {
      width: 1000,
      height: 700,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('connecting again after trusting a certificate opens at the size of the area', async () => {
  area = { width: 1000, height: 700 }
  const view = start(true, true)
  await view.fail({
    kind: 'hostKeyUnknown',
    message: { host: 'rdp/h', port: 3389, fingerprint: 'SHA256:aa' },
  })
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Trust' }))
  })

  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(view.driver.open).toHaveBeenLastCalledWith(
    expect.any(Function),
    {
      width: 1000,
      height: 700,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('a view inside a hidden tab opens unmeasured', async () => {
  area = { width: 1000, height: 700 }
  const fake = fakeDriver(true)
  render(
    <div className="hidden">
      <RemoteDesktopView tabId="t1" driver={fake.driver} active={false} />
    </div>,
  )

  expect(fake.driver.open).toHaveBeenCalledWith(
    expect.any(Function),
    {
      width: 0,
      height: 0,
      scale: 100,
    },
    expect.any(AbortSignal),
  )
})

test('the toolbar keeps its place but is out of reach until the session is connected', async () => {
  const view = start()
  const toolbar = view.container.querySelector('[data-remote-toolbar]')
  expect(toolbar).toHaveClass('invisible')
  expect(toolbar).toHaveAttribute('aria-hidden', 'true')
  expect(toolbar).toHaveAttribute('inert')
  expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull()

  await view.open()
  expect(toolbar).not.toHaveClass('invisible')
  expect(toolbar).not.toHaveAttribute('aria-hidden', 'true')
  expect(toolbar).not.toHaveAttribute('inert')
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument()

  await view.emit({ kind: 'closed', reason: 'gone' })
  expect(toolbar).toHaveClass('invisible')
  expect(toolbar).toHaveAttribute('inert')
})

test('the message covers the view without leaving room for the area to shrink', async () => {
  const view = start()
  const message = screen.getByText('Connecting…').parentElement
  expect(message).toHaveClass('absolute', 'inset-0')
  expect(view.container.firstElementChild).toHaveClass('relative')
})

test('Reconnect and Close tab work in the message over a laid-out area', async () => {
  const view = await connect(true, true)
  await view.emit({ kind: 'closed', reason: 'gone' })
  fireEvent.click(screen.getByRole('button', { name: 'Close tab' }))
  expect(removeTab).toHaveBeenCalledWith('t1')

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  expect(view.driver.open).toHaveBeenCalledTimes(2)
  expect(screen.getByText('Connecting…')).toBeInTheDocument()
})

test('a change of the display scaling with the same CSS size sends the new size and scale', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  changeRatio(1.5)
  await wait(299)
  expect(view.resize).not.toHaveBeenCalled()
  await wait(1)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1500, height: 1050, scale: 150 }]])
})

test('the scaling is watched at the ratio the window has and again after each change', async () => {
  vi.stubGlobal('devicePixelRatio', 1)
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  expect(ratioQueries.map((query) => query.media)).toEqual(['(resolution: 1dppx)'])

  changeRatio(1.5)
  expect(ratioQueries.map((query) => query.media)).toEqual([
    '(resolution: 1dppx)',
    '(resolution: 1.5dppx)',
  ])
  changeRatio(1)
  await wait(300)
  expect(view.resize).not.toHaveBeenCalled()

  changeRatio(2)
  await wait(300)
  expect(view.resize.mock.calls).toEqual([['s1', { width: 2000, height: 1400, scale: 200 }]])
  expect(ratioListeners()).toBe(1)
})

test('a new scale with the same size in device pixels is still sent', async () => {
  vi.stubGlobal('devicePixelRatio', 1)
  area = { width: 1500, height: 1200 }
  useFakeTimers()
  const view = await connect(true, true)
  area = { width: 1000, height: 800 }
  changeRatio(1.5)
  await wait(300)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1500, height: 1200, scale: 150 }]])
})

test('a scaling change lands with the size changes in one debounce', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  changeRatio(1.5)
  await wait(200)
  resizeTo(1200, 800)
  await wait(299)
  expect(view.resize).not.toHaveBeenCalled()
  await wait(1)

  expect(view.resize.mock.calls).toEqual([['s1', { width: 1800, height: 1200, scale: 150 }]])
})

test('no scaling listener is left at unmount', async () => {
  const view = await connect(true, true)
  expect(ratioListeners()).toBe(1)
  changeRatio(1.5)
  expect(ratioListeners()).toBe(1)
  view.unmount()

  expect(ratioListeners()).toBe(0)
})

test('a driver change moves the scaling listener to the new driver', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  const next = fakeDriver(true)
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  expect(ratioListeners()).toBe(1)

  await act(async () => next.opens[0].resolve(session('s2')))
  changeRatio(1.5)
  await wait(300)
  expect(view.resize).not.toHaveBeenCalled()
  expect(next.resize.mock.calls).toEqual([['s2', { width: 1500, height: 1050, scale: 150 }]])
})

test('no scaling listener is kept for a driver without resize', async () => {
  await connect()
  expect(ratioListeners()).toBe(0)
})

test('a size change waiting at a driver change is not sent to the old driver for the new session', async () => {
  area = { width: 1000, height: 700 }
  useFakeTimers()
  const view = await connect(true, true)
  resizeTo(900, 600)
  await wait(200)
  const next = fakeDriver(true)
  await act(async () => {
    view.rerender(<RemoteDesktopView tabId="t1" driver={next.driver} active />)
  })
  await act(async () => next.opens[0].resolve(session('s2')))
  area = { width: 800, height: 500 }
  await wait(500)

  expect(view.resize).not.toHaveBeenCalled()
  expect(next.resize).not.toHaveBeenCalled()
})

test('a key release before any key press does not use up the lock keys', async () => {
  const view = await connect(true, true)
  fireEvent.keyUp(view.canvas, { code: 'KeyA', key: 'a', ...LOCKS })
  expect(view.locks).not.toHaveBeenCalled()

  fireEvent.keyDown(view.canvas, { code: 'KeyB', key: 'b', ...LOCKS })
  expect(view.locks.mock.calls).toEqual([['s1', { caps: true, num: true, scroll: false }]])
})
