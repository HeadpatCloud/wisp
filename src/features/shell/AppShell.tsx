import type { ReactNode } from 'react'

export function AppShell({ main }: { main: ReactNode }) {
  return (
    <div className="flex h-screen w-screen overflow-hidden">
      <main data-testid="main" className="min-w-0 flex-1">
        {main}
      </main>
    </div>
  )
}
