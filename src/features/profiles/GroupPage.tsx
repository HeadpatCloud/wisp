import { useEffect, useState } from 'react'
import type { IconRef } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import { nextGroupOrder } from '@/lib/profiles'
import { useProfileStore } from '@/stores/profileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { IconPicker } from './IconPicker'

export function GroupPage({ groupId, tabId }: { groupId: string | null; tabId: string }) {
  const saveGroup = useProfileStore((s) => s.saveGroup)
  const groups = useProfileStore((s) => s.groups)
  const removeTab = useSessionStore((s) => s.removeTab)
  const group = groupId ? (groups.find((g) => g.id === groupId) ?? null) : null
  const [name, setName] = useState('')
  const [iconRef, setIconRef] = useState<IconRef>({ kind: 'builtin', name: 'cloud' })

  useEffect(() => {
    setName(group?.name ?? '')
    setIconRef(group?.icon ?? { kind: 'builtin', name: 'cloud' })
  }, [group])

  async function onSave() {
    if (!name.trim()) return
    await saveGroup({
      id: group?.id ?? crypto.randomUUID(),
      name: name.trim(),
      parentId: group?.parentId ?? null,
      icon: iconRef,
      order: group?.order ?? nextGroupOrder(groups),
    })
    removeTab(tabId)
  }

  return (
    <PageShell
      title={group ? 'Edit group' : 'New group'}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
            Cancel
          </Button>
          <Button type="button" onClick={onSave}>
            Save
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="group-name">Name</Label>
          <Input id="group-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <IconPicker value={iconRef} onChange={setIconRef} />
      </div>
    </PageShell>
  )
}
