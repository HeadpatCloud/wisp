import { open } from '@tauri-apps/plugin-dialog'
import { Upload } from 'lucide-react'
import type { IconRef } from '@/bindings'
import { Label } from '@/components/ui/label'
import { importIcon } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { BUILTIN_ICONS } from './icons'
import { ProfileIcon } from './ProfileIcon'

export function IconPicker({
  value,
  onChange,
}: {
  value: IconRef
  onChange: (ref: IconRef) => void
}) {
  return (
    <div className="space-y-1">
      <Label>Icon</Label>
      <div className="flex flex-wrap gap-1">
        {BUILTIN_ICONS.map(({ name, Icon }) => (
          <button
            key={name}
            type="button"
            aria-label={`icon ${name}`}
            onClick={() => onChange({ kind: 'builtin', name })}
            className={cn(
              'rounded p-2 hover:bg-muted',
              value.kind === 'builtin' && value.name === name && 'bg-muted ring-1 ring-primary',
            )}
          >
            <Icon className="size-4" />
          </button>
        ))}
        <button
          type="button"
          aria-label="upload custom icon"
          onClick={async () => {
            const file = await open({
              filters: [
                { name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'svg', 'webp', 'gif'] },
              ],
            })
            if (typeof file === 'string') {
              const path = await importIcon(file)
              onChange({ kind: 'custom', path })
            }
          }}
          className={cn(
            'rounded p-2 hover:bg-muted',
            value.kind === 'custom' && 'bg-muted ring-1 ring-primary',
          )}
        >
          {value.kind === 'custom' ? (
            <ProfileIcon icon={value} className="size-4" />
          ) : (
            <Upload className="size-4" />
          )}
        </button>
      </div>
    </div>
  )
}
