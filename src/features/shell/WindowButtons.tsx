import { getCurrentWindow } from '@tauri-apps/api/window'
import { Copy, Minus, Square, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'

const button =
  'flex w-12 items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground'

export function WindowButtons() {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    const win = getCurrentWindow()
    const refresh = () => {
      win.isMaximized().then(setMaximized).catch(console.error)
    }
    refresh()
    const unlisten = win.onResized(refresh)
    return () => {
      unlisten.then((f) => f()).catch(console.error)
    }
  }, [])

  return (
    <div className="flex items-stretch">
      <button
        type="button"
        aria-label="Minimize"
        className={button}
        onClick={() => getCurrentWindow().minimize()}
      >
        <Minus className="size-4" aria-hidden />
      </button>
      <button
        type="button"
        aria-label={maximized ? 'Restore' : 'Maximize'}
        className={button}
        onClick={() => getCurrentWindow().toggleMaximize()}
      >
        {maximized ? (
          <Copy className="size-3.5" aria-hidden />
        ) : (
          <Square className="size-3.5" aria-hidden />
        )}
      </button>
      <button
        type="button"
        aria-label="Close"
        className={cn(button, 'hover:bg-destructive hover:text-destructive-foreground')}
        onClick={() => getCurrentWindow().close()}
      >
        <X className="size-4" aria-hidden />
      </button>
    </div>
  )
}
