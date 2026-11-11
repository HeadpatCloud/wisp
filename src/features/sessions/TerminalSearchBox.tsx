import { ChevronDown, ChevronUp, X } from 'lucide-react'
import { type RefObject, useState } from 'react'
import { Input } from '@/components/ui/input'
import type { TerminalSearch } from './TerminalView'

// Only claim Ctrl+F from the terminal itself: other inputs in the pane keep it, and
// Shift+Ctrl+F still reaches the remote program.
export function isFindChord(ev: React.KeyboardEvent): boolean {
  const t = ev.target as HTMLElement | null
  if (!t?.classList.contains('xterm-helper-textarea')) return false
  return (ev.ctrlKey || ev.metaKey) && !ev.shiftKey && ev.key.toLowerCase() === 'f'
}

export function TerminalSearchBox({
  search,
  onClose,
}: {
  search: RefObject<TerminalSearch | null>
  onClose: () => void
}) {
  const [query, setQuery] = useState('')
  return (
    <div className="absolute top-1 right-1 z-10 flex items-center gap-0.5 rounded border border-border bg-background p-1 shadow-md">
      <Input
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            if (e.shiftKey) search.current?.findPrevious(query)
            else search.current?.findNext(query)
          } else if (e.key === 'Escape') {
            onClose()
          }
        }}
        placeholder="Find…"
        className="h-7 w-44 text-xs"
      />
      <button
        type="button"
        aria-label="Previous match"
        onClick={() => search.current?.findPrevious(query)}
        className="rounded p-1 hover:bg-muted"
      >
        <ChevronUp className="size-4" />
      </button>
      <button
        type="button"
        aria-label="Next match"
        onClick={() => search.current?.findNext(query)}
        className="rounded p-1 hover:bg-muted"
      >
        <ChevronDown className="size-4" />
      </button>
      <button
        type="button"
        aria-label="Close search"
        onClick={onClose}
        className="rounded p-1 hover:bg-muted"
      >
        <X className="size-4" />
      </button>
    </div>
  )
}
