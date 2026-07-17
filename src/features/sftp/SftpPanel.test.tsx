import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  listDir: vi.fn(),
  onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
}))
vi.mock('@/lib/sftp', () => ({
  listDir: m.listDir,
  download: vi.fn(),
  downloadTo: vi.fn(),
  upload: vi.fn(),
  exists: vi.fn(),
  mkdir: vi.fn(),
  rename: vi.fn(),
  remove: vi.fn(),
  basename: (p: string) => p,
}))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: m.onDragDropEvent }),
}))

import { SftpPanel } from './SftpPanel'

beforeEach(() => {
  vi.clearAllMocks()
  m.listDir.mockResolvedValue([
    { name: 'docs', path: '/docs', isDir: true, isSymlink: false, size: 0, modified: null },
  ])
})

test('lists the initial directory and registers a drag-drop listener', async () => {
  render(<SftpPanel sessionId="s1" />)
  expect(await screen.findByText('docs')).toBeInTheDocument()
  expect(m.listDir).toHaveBeenCalledWith('s1', '.')
  await waitFor(() => expect(m.onDragDropEvent).toHaveBeenCalled())
})
