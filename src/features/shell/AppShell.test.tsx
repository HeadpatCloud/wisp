import { render, screen } from '@testing-library/react'
import { expect, test } from 'vitest'
import { AppShell } from './AppShell'

test('renders the main region', () => {
  render(<AppShell main={<div>body</div>} />)
  expect(screen.getByTestId('main')).toHaveTextContent('body')
})
