import { check, type Update } from '@tauri-apps/plugin-updater'
import { hostsAskingToClose } from '@/features/sessions/tabHosts'
import {
  checkForUpdate,
  downloadUpdate,
  installUpdate,
  isVersionSkipped,
  skipVersion,
} from '@/lib/updater'
import { confirm } from '@/stores/confirmStore'
import { notify, useNoticeStore } from '@/stores/noticeStore'

const release = (update: Update) => {
  update.close().catch(() => {})
}

async function restart(update: Update) {
  const names = hostsAskingToClose()
  if (
    names &&
    !(await confirm({
      title: 'Restart Wisp?',
      body: `Tabs of ${names} are tagged to ask first. Anything still running in them stops.`,
      confirmLabel: 'Restart now',
      destructive: true,
    }))
  ) {
    ready(update)
    return
  }
  try {
    await installUpdate(update)
  } catch (e) {
    notify({ title: 'The update failed', body: String(e), tone: 'error' })
  }
}

function ready(update: Update) {
  notify({
    title: `Wisp ${update.version} is ready.`,
    body: 'It installs when you choose Restart now.',
    sticky: true,
    actions: [
      { label: 'Restart now', run: () => void restart(update) },
      {
        label: 'Skip',
        run: () => {
          skipVersion(update.version)
          release(update)
        },
      },
      { label: 'Later', run: () => release(update) },
    ],
    onClose: () => release(update),
  })
}

// In the background unless the user asked, when the download shows its progress.
async function download(update: Update, shown: boolean) {
  const id = shown
    ? notify({ title: `Downloading Wisp ${update.version}`, body: 'Starting', sticky: true })
    : null
  try {
    await downloadUpdate(update, (percent) => {
      if (id) useNoticeStore.getState().update(id, { body: percent === null ? '' : `${percent}%` })
    })
  } catch (e) {
    notify({
      title: `Wisp ${update.version} couldn't be downloaded.`,
      body: String(e),
      tone: 'error',
    })
    release(update)
    return
  } finally {
    if (id) useNoticeStore.getState().dismiss(id)
  }
  ready(update)
}

// StrictMode runs the start effect twice; one download is enough.
let started = false

export async function startUpdateCheck() {
  if (started) return
  started = true
  const update = await checkForUpdate()
  if (!update) return
  if (isVersionSkipped(update.version)) {
    release(update)
    return
  }
  await download(update, false)
}

// From the Wisp menu: says how it went, and offers a skipped version again.
export async function checkForUpdates() {
  let update: Update | null
  try {
    update = await check()
  } catch (e) {
    notify({ title: "Couldn't check for updates.", body: String(e), tone: 'error' })
    return
  }
  if (update) await download(update, true)
  else notify({ title: 'Wisp is up to date.' })
}
