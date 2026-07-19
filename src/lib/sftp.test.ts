import { expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  sftpUpload: vi.fn().mockResolvedValue({ status: 'ok', data: null }),
}))
vi.mock('@/bindings', () => ({ commands: { sftpUpload: m.sftpUpload } }))
vi.mock('@tauri-apps/api/core', () => ({ Channel: class {} }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))

import { basename, upload } from './sftp'

test('basename strips trailing separators', () => {
  expect(basename('/a/b/c.txt')).toBe('c.txt')
  expect(basename('/a/b/')).toBe('b')
  expect(basename('C:\\dir\\f.txt')).toBe('f.txt')
})

// remoteDir arrives with a trailing slash from breadcrumbs and S3-style prefixes.
test.each([
  ['/home/user', '/home/user/f.txt'],
  ['/home/user/', '/home/user/f.txt'],
  ['/', '/f.txt'],
  ['.', './f.txt'],
])('upload joins %s without doubling the separator', async (dir, expected) => {
  m.sftpUpload.mockClear()
  await upload('sid', 'tid', '/local/f.txt', dir, () => {})
  expect(m.sftpUpload).toHaveBeenCalledWith(
    'sid',
    'tid',
    '/local/f.txt',
    expected,
    expect.anything(),
  )
})
