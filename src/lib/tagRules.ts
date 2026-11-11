import type { Host, Tag } from '@/bindings'

export interface TagRules {
  color: string | null
  askBeforeClose: boolean
  askBeforePaste: boolean
  terminalColors: string | null
  // The first tag that asks before closing, named in the question.
  closeTag: string | null
}

// Any tag that asks makes the host ask; a colour and terminal colours come from the first tag,
// in the Tags list's order, that sets them.
export function rulesFor(host: Host | undefined, tags: Tag[]): TagRules {
  const own = host ? tags.filter((t) => (host.tagIds ?? []).includes(t.id)) : []
  const close = own.find((t) => t.askBeforeClose)
  return {
    color: own.find((t) => t.showColor)?.color ?? null,
    askBeforeClose: !!close,
    askBeforePaste: own.some((t) => t.askBeforePaste),
    terminalColors: own.find((t) => t.terminalColors)?.terminalColors ?? null,
    closeTag: close?.name ?? null,
  }
}

// Text with a line break runs line by line as it lands, so a host that asks first shows it.
export async function guardedPaste(
  text: string,
  ask: boolean,
  confirmLines: (text: string) => Promise<boolean>,
  paste: (text: string) => void,
): Promise<void> {
  if (ask && /[\r\n]/.test(text) && !(await confirmLines(text))) return
  paste(text)
}
