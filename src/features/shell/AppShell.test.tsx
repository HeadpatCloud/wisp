import { render, screen } from '@testing-library/react'
import { expect, test } from 'vitest'
import { AppShell } from './AppShell'

test('renders sidebar and main regions', () => {
  render(<AppShell sidebar={<div>side</div>} main={<div>body</div>} />)
  expect(screen.getByTestId('sidebar')).toHaveTextContent('side')
  expect(screen.getByTestId('main')).toHaveTextContent('body')
})
