import { useState } from 'react'
import { Input } from '@/components/ui/input'
import { TAG_COLORS, tagColor } from '@/lib/tagColors'
import { cn } from '@/lib/utils'
import { useHostStore } from '@/stores/hostStore'

// Pick existing tags, or type a new name and press Enter to make one.
export function TagsField({
  value,
  onChange,
}: {
  value: string[]
  onChange: (ids: string[]) => void
}) {
  const tags = useHostStore((s) => s.tags)
  const saveTag = useHostStore((s) => s.saveTag)
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const toggle = (id: string) =>
    onChange(value.includes(id) ? value.filter((t) => t !== id) : [...value, id])

  const create = () => {
    const trimmed = name.trim()
    if (!trimmed) return
    const existing = tags.find((t) => t.name.toLowerCase() === trimmed.toLowerCase())
    if (existing) {
      if (!value.includes(existing.id)) onChange([...value, existing.id])
      setName('')
      return
    }
    const id = crypto.randomUUID()
    saveTag({
      id,
      name: trimmed,
      color: 'grey',
      showColor: false,
      askBeforeClose: false,
      askBeforePaste: false,
      terminalColors: null,
    })
      .then(() => {
        onChange([...value, id])
        setName('')
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
  }

  return (
    <fieldset className="col-span-full flex flex-col gap-2">
      <legend className="mb-1 font-medium text-sm">Tags</legend>
      <div className="flex flex-wrap items-center gap-1.5">
        {tags.map((t) => (
          <button
            key={t.id}
            type="button"
            aria-pressed={value.includes(t.id)}
            onClick={() => toggle(t.id)}
            className={cn(
              'flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs',
              value.includes(t.id) ? 'border-primary bg-accent' : 'border-border hover:bg-muted',
            )}
          >
            {t.showColor && (
              <span className={cn('size-1.5 rounded-full', tagColor(t.color)?.dot)} />
            )}
            {t.name}
          </button>
        ))}
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== 'Enter') return
            e.preventDefault()
            create()
          }}
          placeholder="New tag"
          aria-label="New tag"
          className="h-7 w-32 text-xs"
        />
      </div>
      {error && <p className="text-destructive text-xs">{error}</p>}
    </fieldset>
  )
}

export function HostColorField({
  value,
  onChange,
}: {
  value: string | null
  onChange: (c: string | null) => void
}) {
  return (
    <fieldset className="col-span-full flex flex-col gap-1">
      <legend className="mb-1 font-medium text-sm">Colour</legend>
      <div className="flex flex-wrap gap-1.5">
        <button
          type="button"
          aria-label="No colour"
          aria-pressed={value === null}
          onClick={() => onChange(null)}
          className={cn(
            'size-6 rounded-full border border-border bg-muted ring-offset-2 ring-offset-background',
            value === null && 'ring-2 ring-ring',
          )}
        />
        {TAG_COLORS.map((c) => (
          <button
            key={c.name}
            type="button"
            aria-label={c.label}
            aria-pressed={value === c.name}
            onClick={() => onChange(c.name)}
            className={cn(
              'size-6 rounded-full ring-offset-2 ring-offset-background',
              c.dot,
              value === c.name && 'ring-2 ring-ring',
            )}
          />
        ))}
      </div>
    </fieldset>
  )
}
