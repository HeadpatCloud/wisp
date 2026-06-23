import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({ readIcon: vi.fn().mockResolvedValue('data:image/png;base64,AAA') }))
vi.mock('@/lib/icons', () => ({ readIcon: m.readIcon, importIcon: vi.fn() }))

import { ProfileIcon } from './ProfileIcon'

beforeEach(() => vi.clearAllMocks())

test('renders a lucide svg for a builtin icon', () => {
  const { container } = render(
    <ProfileIcon icon={{ kind: 'builtin', name: 'database' }} className="size-4" />,
  )
  expect(container.querySelector('svg')).toBeInTheDocument()
})

test('renders an img with the data URL for a custom icon', async () => {
  render(<ProfileIcon icon={{ kind: 'custom', path: 'icons/x.png' }} className="size-4" />)
  await waitFor(() =>
    expect(screen.getByRole('img')).toHaveAttribute('src', 'data:image/png;base64,AAA'),
  )
  expect(m.readIcon).toHaveBeenCalledWith('icons/x.png')
})
