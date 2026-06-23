import { render, screen } from '@testing-library/react'
import { expect, test, vi } from 'vitest'
import { ErrorBoundary } from './ErrorBoundary'

function Boom(): never {
  throw new Error('kaboom')
}

test('renders children when nothing throws', () => {
  render(
    <ErrorBoundary>
      <div>all good</div>
    </ErrorBoundary>,
  )
  expect(screen.getByText('all good')).toBeInTheDocument()
})

test('shows a fallback with the error message when a child throws', () => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined)
  render(
    <ErrorBoundary>
      <Boom />
    </ErrorBoundary>,
  )
  expect(screen.getByText(/something went wrong/i)).toBeInTheDocument()
  expect(screen.getByText(/kaboom/)).toBeInTheDocument()
})
