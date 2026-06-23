import type { ReactNode } from 'react'

interface AppShellProps {
  sidebar: ReactNode
  main: ReactNode
}

export function AppShell({ sidebar, main }: AppShellProps) {
  return (
    <div className="flex h-screen w-screen overflow-hidden">
      <aside data-testid="sidebar" className="w-72 shrink-0 border-border border-r bg-sidebar">
        {sidebar}
      </aside>
      <main data-testid="main" className="min-w-0 flex-1">
        {main}
      </main>
    </div>
  )
}
