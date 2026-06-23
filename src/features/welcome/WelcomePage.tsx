import { Download, Plus, Settings } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useSessionStore } from '@/stores/sessionStore'

const TIPS = [
  'Create a profile with the + in the sidebar, then double-click it to connect.',
  'First connection to a host asks you to trust its key (TOFU).',
  'Import your existing hosts from ~/.ssh/config.',
  'Open SFTP or tunnels from the toolbar inside a connected pane.',
]

export function WelcomePage() {
  const openView = useSessionStore((s) => s.openView)
  return (
    <div className="flex h-full flex-col items-center justify-center gap-8 p-8 text-center">
      <div className="flex flex-col items-center gap-4">
        <img src="/wisp-icon.svg" alt="" className="size-24 drop-shadow-md" />
        <div>
          <h1 className="font-semibold text-4xl tracking-tight">Wisp</h1>
          <p className="mt-1.5 text-muted-foreground text-sm">Fast, secure SSH - free and open.</p>
        </div>
      </div>
      <div className="flex flex-wrap justify-center gap-3">
        <Button
          type="button"
          onClick={() => openView({ kind: 'profile-editor', profileId: null }, 'New profile')}
        >
          <Plus className="size-4" /> New SSH profile
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => openView({ kind: 'import' }, 'Import')}
        >
          <Download className="size-4" /> Import ~/.ssh/config
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => openView({ kind: 'settings' }, 'Settings')}
        >
          <Settings className="size-4" /> Settings
        </Button>
      </div>
      <ul className="max-w-md space-y-2 text-left text-muted-foreground text-sm">
        {TIPS.map((tip) => (
          <li key={tip} className="flex gap-2">
            <span className="text-foreground">-</span>
            {tip}
          </li>
        ))}
      </ul>
    </div>
  )
}
