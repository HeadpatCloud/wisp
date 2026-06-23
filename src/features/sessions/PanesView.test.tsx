import { render, screen } from '@testing-library/react'
import { expect, test, vi } from 'vitest'

vi.mock('./PaneView', () => ({
  PaneView: ({ sessionId }: { sessionId: string }) => <div data-testid="pane">{sessionId}</div>,
}))

import type { SessionTab } from '@/stores/sessionStore'
import { PanesView } from './PanesView'

test('renders one pane per session id in order', () => {
  const tab: SessionTab = {
    id: 't1',
    kind: 'session',
    sessionIds: ['a', 'b', 'c'],
    direction: 'horizontal',
    activePaneId: 'a',
  }
  render(<PanesView tab={tab} />)
  expect(screen.getAllByTestId('pane').map((p) => p.textContent)).toEqual(['a', 'b', 'c'])
})
