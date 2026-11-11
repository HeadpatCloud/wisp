import { useState } from 'react'

// Enter or leaving the field saves a name that isn't empty; Escape or an empty name cancels.
export function InlineName({
  initial,
  label,
  onSave,
  onCancel,
}: {
  initial: string
  label: string
  onSave: (name: string) => void
  onCancel: () => void
}) {
  const [value, setValue] = useState(initial)
  const finish = () => {
    const name = value.trim()
    if (name) onSave(name)
    else onCancel()
  }
  return (
    <input
      // biome-ignore lint/a11y/noAutofocus: the field appears because the user asked to name something
      autoFocus
      aria-label={label}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={finish}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') {
          e.preventDefault()
          finish()
        } else if (e.key === 'Escape') {
          e.preventDefault()
          onCancel()
        }
      }}
      className="h-7 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm"
    />
  )
}
