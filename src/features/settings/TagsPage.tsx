import { useState } from 'react'
import type { Tag } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { BUILT_IN_SCHEMES } from '@/lib/schemes'
import { TAG_COLORS, tagColor } from '@/lib/tagColors'
import { BUILT_IN_THEMES } from '@/lib/themes'
import { cn } from '@/lib/utils'
import { confirm } from '@/stores/confirmStore'
import { useHostStore } from '@/stores/hostStore'
import { useThemeStore } from '@/stores/themeStore'
import { Choice, Row, Toggle } from './controls'

const errorText = (e: unknown) =>
  e && typeof e === 'object' && 'message' in e ? String(e.message) : String(e)

const NONE = 'none'

function TagEditor({ tag, onDone }: { tag: Tag | null; onDone: () => void }) {
  const saveTag = useHostStore((s) => s.saveTag)
  const deleteTag = useHostStore((s) => s.deleteTag)
  const hosts = useHostStore((s) => s.hosts)
  const userThemes = useThemeStore((s) => s.userThemes)
  const schemes = useThemeStore((s) => s.schemes)
  const [draft, setDraft] = useState<Tag>(
    tag ?? {
      id: crypto.randomUUID(),
      name: '',
      color: 'red',
      showColor: true,
      askBeforeClose: false,
      askBeforePaste: false,
      terminalColors: null,
    },
  )
  const [error, setError] = useState<string | null>(null)
  const set = (patch: Partial<Tag>) => setDraft({ ...draft, ...patch })
  const colourOptions: [string, string][] = [
    [NONE, 'The connection’s or the Terminal setting’s'],
    ...[...BUILT_IN_SCHEMES, ...schemes].map((s): [string, string] => [s.id, s.name]),
    ...[...BUILT_IN_THEMES, ...userThemes].map((t): [string, string] => [
      `theme:${t.id}`,
      `${t.name} (theme)`,
    ]),
  ]

  const remove = async () => {
    if (!tag) return
    const used = hosts.filter((h) => (h.tagIds ?? []).includes(tag.id)).length
    const ok = await confirm({
      title: `Delete ${tag.name}?`,
      body: used
        ? `It comes off ${used === 1 ? '1 host' : `${used} hosts`}, and their rules go with it.`
        : 'No host uses it.',
      confirmLabel: 'Delete tag',
      destructive: true,
    })
    if (!ok) return
    await deleteTag(tag.id)
    onDone()
  }

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border p-3">
      <Row id={`tag-${draft.id}-name`} label="Name">
        <Input
          id={`tag-${draft.id}-name`}
          value={draft.name}
          onChange={(e) => set({ name: e.target.value })}
        />
      </Row>
      <fieldset className="flex flex-col gap-1">
        <legend className="mb-1 font-medium text-sm">Colour</legend>
        <div className="flex flex-wrap gap-1.5">
          {TAG_COLORS.map((c) => (
            <button
              key={c.name}
              type="button"
              aria-label={c.label}
              aria-pressed={draft.color === c.name}
              onClick={() => set({ color: c.name })}
              className={cn(
                'size-6 rounded-full ring-offset-2 ring-offset-background',
                c.dot,
                draft.color === c.name && 'ring-2 ring-ring',
              )}
            />
          ))}
        </div>
      </fieldset>
      <Toggle
        label="Show the colour"
        checked={draft.showColor}
        onChange={(v) => set({ showColor: v })}
        hint="On its hosts' cards and tabs, and around the window while one of their tabs is open."
      />
      <Toggle
        label="Ask before closing"
        checked={draft.askBeforeClose}
        onChange={(v) => set({ askBeforeClose: v })}
        hint="Closing a tab of these hosts, or Wisp with one open, asks first."
      />
      <Toggle
        label="Ask before pasting several lines"
        checked={draft.askBeforePaste}
        onChange={(v) => set({ askBeforePaste: v })}
        hint="Shows the lines first, since each one may run as soon as it lands."
      />
      <Choice
        id={`tag-${draft.id}-colors`}
        label="Terminal colours"
        value={draft.terminalColors ?? NONE}
        onChange={(v) => set({ terminalColors: v === NONE ? null : v })}
        options={colourOptions}
        hint="A connection's own colours still win."
      />
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        {tag && (
          <Button
            type="button"
            variant="ghost"
            onClick={() => remove().catch((e: unknown) => setError(errorText(e)))}
          >
            Delete tag…
          </Button>
        )}
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          type="button"
          disabled={!draft.name.trim()}
          onClick={() =>
            saveTag({ ...draft, name: draft.name.trim() })
              .then(onDone)
              .catch((e: unknown) => setError(errorText(e)))
          }
        >
          Save
        </Button>
      </div>
    </div>
  )
}

export function TagsPage() {
  const tags = useHostStore((s) => s.tags)
  const hosts = useHostStore((s) => s.hosts)
  const [editing, setEditing] = useState<string | null>(null)
  const count = (id: string) => hosts.filter((h) => (h.tagIds ?? []).includes(id)).length

  return (
    <div className="flex max-w-2xl flex-col gap-3">
      <div className="flex items-center justify-between">
        <p className="text-muted-foreground text-sm">
          Tags group hosts across folders and carry rules: a colour, questions before closing or
          pasting, and terminal colours.
        </p>
        <Button type="button" variant="outline" size="sm" onClick={() => setEditing('new')}>
          New tag
        </Button>
      </div>
      {editing === 'new' && <TagEditor tag={null} onDone={() => setEditing(null)} />}
      {tags.length === 0 && editing !== 'new' && (
        <p className="text-muted-foreground text-sm">No tags yet.</p>
      )}
      {tags.map((t) =>
        editing === t.id ? (
          <TagEditor key={t.id} tag={t} onDone={() => setEditing(null)} />
        ) : (
          <div
            key={t.id}
            className="flex items-center gap-3 rounded-lg border border-border p-3 text-sm"
          >
            <span className={cn('size-2.5 shrink-0 rounded-full', tagColor(t.color)?.dot)} />
            <span className="min-w-0 flex-1 truncate font-medium">{t.name}</span>
            <span className="text-muted-foreground text-xs">
              {count(t.id) === 1 ? '1 host' : `${count(t.id)} hosts`}
            </span>
            <Button type="button" variant="outline" size="sm" onClick={() => setEditing(t.id)}>
              Edit
            </Button>
          </div>
        ),
      )}
    </div>
  )
}
