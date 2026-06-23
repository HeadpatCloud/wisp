import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  listDir: vi.fn(),
  onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
}))
vi.mock('@/lib/ftp', () => ({
  listDir: m.listDir,
  download: vi.fn(),
  upload: vi.fn(),
  exists: vi.fn(),
  mkdir: vi.fn(),
  rename: vi.fn(),
  remove: vi.fn(),
}))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: m.onDragDropEvent }),
}))

import { FtpPanel } from './FtpPanel'

beforeEach(() => {
  vi.clearAllMocks()
  m.listDir.mockResolvedValue([
    { name: 'pub', path: '/pub', isDir: true, isSymlink: false, size: 0, modified: null },
  ])
})

test('lists the initial directory and registers a drag-drop listener', async () => {
  render(<FtpPanel sessionId="f1" />)
  expect(await screen.findByText('pub')).toBeInTheDocument()
  expect(m.listDir).toHaveBeenCalledWith('f1', '.')
  await waitFor(() => expect(m.onDragDropEvent).toHaveBeenCalled())
})
