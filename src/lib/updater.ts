import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { commands } from '@/bindings'
import { unwrap } from '@/lib/ipc'

const SKIP_KEY = 'wisp.updater.skippedVersion'

export async function checkForUpdate(): Promise<Update | null> {
  try {
    return await check()
  } catch {
    // No release yet, offline, or running unpackaged (dev/web) - nothing to do.
    return null
  }
}

export function isVersionSkipped(version: string): boolean {
  return localStorage.getItem(SKIP_KEY) === version
}

export function skipVersion(version: string): void {
  localStorage.setItem(SKIP_KEY, version)
}

export async function installUpdate(
  update: Update,
  onProgress?: (percent: number | null) => void,
): Promise<void> {
  let downloaded = 0
  let total: number | null = null
  await update.download((event) => {
    if (event.event === 'Started') {
      total = event.data.contentLength ?? null
    } else if (event.event === 'Progress') {
      downloaded += event.data.chunkLength
      onProgress?.(total ? Math.round((downloaded / total) * 100) : null)
    }
  })
  // On Windows the install ends the app at once, without the exit's close of the RDP sessions,
  // and a dropped one leaves the user's Windows session blocked.
  unwrap(await commands.rdpCloseAll())
  await update.install()
  await relaunch()
}
