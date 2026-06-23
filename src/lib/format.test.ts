import { expect, test } from 'vitest'
import { formatBytes } from './format'

test('formats bytes into B/KB/MB/GB', () => {
  expect(formatBytes(0)).toBe('0 B')
  expect(formatBytes(512)).toBe('512 B')
  expect(formatBytes(2048)).toBe('2.0 KB')
  expect(formatBytes(1572864)).toBe('1.5 MB')
  expect(formatBytes(3 * 1024 ** 3)).toBe('3.0 GB')
})
