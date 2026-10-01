import { useEffect, useRef } from 'react'
import type { Tri } from './review'

export function TriCheckbox({
  state,
  label,
  onChange,
  id,
}: {
  state: Tri
  label: string
  onChange: (on: boolean) => void
  id?: string
}) {
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = state === 'some'
  }, [state])
  return (
    <input
      ref={ref}
      id={id}
      type="checkbox"
      aria-label={label}
      checked={state === 'all'}
      onChange={(e) => onChange(e.target.checked)}
    />
  )
}
