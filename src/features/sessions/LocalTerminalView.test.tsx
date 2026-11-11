import { fireEvent, render, screen } from '@testing-library/react'
import { act } from 'react'
import { beforeEach, expect, test, vi } from 'vitest'

vi.mock('@/lib/local', () => ({
  openLocal: vi.fn().mockResolvedValue('local-1'),
  writeLocal: vi.fn(),
  resizeLocal: vi.fn(),
  closeLocal: vi.fn(),
}))
vi.mock('./TerminalView', () => ({
  TerminalView: ({ settings }: { settings: { fontSize: number } }) => (
    <div data-testid="terminal">{settings.fontSize}</div>
  ),
}))

import { useSessionStore } from '@/stores/sessionStore'
import { useSettingsStore } from '@/stores/settingsStore'
import { LocalTerminalView } from './LocalTerminalView'

const zoomOf = (id: string) => useSessionStore.getState().panes[id]?.zoom
const local = (id: string, zoom?: number) => ({
  id,
  kind: 'local' as const,
  title: 'Local shell',
  status: 'connecting' as const,
  reconnectNonce: 0,
  connectionId: null,
  program: null,
  startFolder: null,
  zoom,
})

beforeEach(() => {
  useSettingsStore.setState({ settings: { ...useSettingsStore.getState().settings, fontSize: 14 } })
  useSessionStore.setState({
    tabs: [],
    panes: { l1: local('l1'), l2: local('l2', 5) },
    activeTabId: null,
  })
})

test('the font size follows the zoom of its own pane', () => {
  render(<LocalTerminalView tabId="t1" paneId="l1" />)
  expect(screen.getByTestId('terminal')).toHaveTextContent('14')
  act(() => useSessionStore.getState().setZoom('l1', 3))
  expect(screen.getByTestId('terminal')).toHaveTextContent('17')
  act(() => useSessionStore.getState().setZoom('l1', -6))
  expect(screen.getByTestId('terminal')).toHaveTextContent('8')
  act(() => useSessionStore.getState().setZoom('l1', 0))
  expect(screen.getByTestId('terminal')).toHaveTextContent('14')
})

test('the font size never drops below 6', () => {
  useSettingsStore.setState({ settings: { ...useSettingsStore.getState().settings, fontSize: 8 } })
  render(<LocalTerminalView tabId="t1" paneId="l1" />)
  act(() => useSessionStore.getState().setZoom('l1', -6))
  expect(screen.getByTestId('terminal')).toHaveTextContent('6')
})

test('Ctrl+wheel zooms this pane only and a plain wheel does not', () => {
  render(<LocalTerminalView tabId="t1" paneId="l1" />)
  const terminal = screen.getByTestId('terminal')
  expect(fireEvent.wheel(terminal, { deltaY: -100, ctrlKey: true })).toBe(false)
  fireEvent.wheel(terminal, { deltaY: -100, ctrlKey: true })
  expect(zoomOf('l1')).toBe(2)
  fireEvent.wheel(terminal, { deltaY: 100, ctrlKey: true })
  expect(zoomOf('l1')).toBe(1)
  fireEvent.wheel(terminal, { deltaY: -100 })
  expect(zoomOf('l1')).toBe(1)
  expect(zoomOf('l2')).toBe(5)
  expect(terminal).toHaveTextContent('15')
})
