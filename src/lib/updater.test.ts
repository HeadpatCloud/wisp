import { beforeEach, expect, test, vi } from 'vitest'

const m = vi.hoisted(() => ({
  steps: [] as string[],
  relaunch: vi.fn(),
  rdpCloseAll: vi.fn(),
}))
vi.mock('@/bindings', () => ({ commands: { rdpCloseAll: m.rdpCloseAll } }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: m.relaunch }))
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))

import type { DownloadEvent, Update } from '@tauri-apps/plugin-updater'
import { installUpdate } from './updater'

function update(download: (onEvent: (event: DownloadEvent) => void) => Promise<void>) {
  return {
    download: vi.fn(async (onEvent: (event: DownloadEvent) => void) => {
      m.steps.push('download')
      await download(onEvent)
    }),
    install: vi.fn(async () => {
      m.steps.push('install')
    }),
  } as unknown as Update
}

beforeEach(() => {
  m.steps = []
  m.relaunch.mockReset().mockImplementation(async () => {
    m.steps.push('relaunch')
  })
  m.rdpCloseAll.mockReset().mockImplementation(async () => {
    m.steps.push('close')
    return { status: 'ok', data: null }
  })
})

test('the RDP sessions are closed after the download and before the install', async () => {
  const progress: (number | null)[] = []
  const downloaded = update(async (onEvent) => {
    onEvent({ event: 'Started', data: { contentLength: 200 } })
    onEvent({ event: 'Progress', data: { chunkLength: 50 } })
    onEvent({ event: 'Progress', data: { chunkLength: 150 } })
    onEvent({ event: 'Finished' })
  })
  await installUpdate(downloaded, (percent) => progress.push(percent))
  expect(m.steps).toEqual(['download', 'close', 'install', 'relaunch'])
  expect(progress).toEqual([25, 100])
})

test('a failed download closes nothing and installs nothing', async () => {
  const failed = update(async () => {
    throw new Error('offline')
  })
  await expect(installUpdate(failed)).rejects.toThrow('offline')
  expect(m.steps).toEqual(['download'])
  expect(m.rdpCloseAll).not.toHaveBeenCalled()
})
