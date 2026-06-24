import type { Update } from '@tauri-apps/plugin-updater'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { checkForUpdate, installUpdate, isVersionSkipped, skipVersion } from '@/lib/updater'

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null)
  const [installing, setInstalling] = useState(false)
  const [progress, setProgress] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    checkForUpdate().then((u) => {
      if (!u) return
      if (cancelled || isVersionSkipped(u.version)) {
        u.close().catch(() => {})
        return
      }
      setUpdate(u)
    })
    return () => {
      cancelled = true
    }
  }, [])

  if (!update) return null

  const dismiss = () => {
    update.close().catch(() => {})
    setUpdate(null)
  }
  const skip = () => {
    skipVersion(update.version)
    dismiss()
  }
  const install = async () => {
    setInstalling(true)
    setError(null)
    try {
      await installUpdate(update, setProgress)
    } catch (e) {
      setError(String(e))
      setInstalling(false)
    }
  }

  return (
    <div className="fixed right-4 bottom-4 z-50 w-80 rounded-lg border border-border bg-background p-4 shadow-lg">
      <h3 className="font-medium text-sm">Update available</h3>
      <p className="mt-1 text-muted-foreground text-xs">
        Wisp {update.version} is ready to install (you have {update.currentVersion}).
      </p>
      {update.body && (
        <p className="mt-2 max-h-24 overflow-y-auto whitespace-pre-line text-muted-foreground text-xs">
          {update.body}
        </p>
      )}
      {error && <p className="mt-2 text-destructive text-xs">{error}</p>}
      {installing ? (
        <p className="mt-3 text-xs">
          {progress === null ? 'Downloading...' : `Downloading ${progress}%`}
        </p>
      ) : (
        <div className="mt-3 flex gap-2">
          <Button size="sm" onClick={install}>
            Update now
          </Button>
          <Button size="sm" variant="ghost" onClick={skip}>
            Skip
          </Button>
          <Button size="sm" variant="ghost" onClick={dismiss}>
            Later
          </Button>
        </div>
      )}
    </div>
  )
}
