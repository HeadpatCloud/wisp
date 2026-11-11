import { openUrl } from '@tauri-apps/plugin-opener'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { importFromFile } from '@/features/transfer/importFromFile'
import { checkForUpdates } from '@/features/updater/updateNotice'
import { useHostsUiStore } from '@/stores/hostsUiStore'

const PROJECT = 'https://github.com/HeadpatCloud/wisp'

// Checkpoint 3 moves this into the custom title bar, and to the menu bar on macOS.
export function WispMenu() {
  const openPage = useHostsUiStore((s) => s.openPage)
  const [about, setAbout] = useState(false)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Wisp menu"
            className="flex h-7 items-center gap-1.5 rounded-md px-2 font-bold text-sm hover:bg-muted"
          >
            <img src="/wisp-icon.svg" alt="" className="size-4" />
            Wisp
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => openPage('sshImport')}>
            Import from ~/.ssh/config
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void importFromFile()}>
            Import a Wisp export…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => openPage('export')}>Export hosts…</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void checkForUpdates()}>
            Check for updates
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setAbout(true)}>About Wisp</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={about} onOpenChange={setAbout}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader className="items-center text-center">
            <img src="/wisp-icon.svg" alt="" className="size-14" />
            <DialogTitle>Wisp</DialogTitle>
            <DialogDescription>Version {__APP_VERSION__}</DialogDescription>
          </DialogHeader>
          <DialogFooter className="sm:justify-center">
            <Button variant="outline" onClick={() => void openUrl(PROJECT)}>
              github.com/HeadpatCloud/wisp
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
