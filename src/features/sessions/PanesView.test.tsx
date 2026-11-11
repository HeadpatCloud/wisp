import { act, render, screen } from '@testing-library/react'
import { useEffect } from 'react'
import { beforeEach, expect, test, vi } from 'vitest'

const life: string[] = []
vi.mock('./PaneView', () => ({
  PaneView: ({ paneId }: { paneId: string }) => {
    useEffect(() => {
      life.push(`mount ${paneId}`)
      return () => {
        life.push(`unmount ${paneId}`)
      }
    }, [paneId])
    return <div data-testid="pane">{paneId}</div>
  },
}))

import { type PanesTab, useSessionStore } from '@/stores/sessionStore'
import { PaneLayer } from './PaneLayer'
import { PanesView } from './PanesView'

const ssh = (id: string) => ({
  id,
  kind: 'ssh' as const,
  title: id,
  status: 'connecting' as const,
  reconnectNonce: 0,
  connectionId: 'c1',
})
const tab = (id: string, paneIds: string[]): PanesTab => ({
  id,
  kind: 'panes',
  paneIds,
  direction: 'horizontal',
  activePaneId: paneIds[0],
})

// What App renders: one PanesView per tab, and the panes themselves once, in the layer.
function Tabs() {
  const tabs = useSessionStore((s) => s.tabs)
  return (
    <>
      {tabs.map((t) =>
        t.kind === 'panes' ? (
          <div key={t.id}>
            <PanesView tab={t} />
          </div>
        ) : null,
      )}
      <PaneLayer />
    </>
  )
}

beforeEach(() => {
  life.length = 0
})

test('renders one pane per pane id in order', () => {
  useSessionStore.setState({
    tabs: [tab('t1', ['a', 'b', 'c'])],
    panes: { a: ssh('a'), b: ssh('b'), c: ssh('c') },
    activeTabId: 't1',
  })
  render(<Tabs />)
  expect(screen.getAllByTestId('pane').map((p) => p.textContent)).toEqual(['a', 'b', 'c'])
})

test('a pane keeps running when its tab is merged into another', () => {
  useSessionStore.setState({
    tabs: [tab('t1', ['a']), tab('t2', ['b'])],
    panes: { a: ssh('a'), b: ssh('b') },
    activeTabId: 't1',
  })
  render(<Tabs />)
  life.length = 0
  act(() => useSessionStore.getState().mergeTab('t2', 't1', 'horizontal'))
  expect(life).toEqual([])
  expect(screen.getAllByTestId('pane').map((p) => p.textContent)).toEqual(['a', 'b'])
})
