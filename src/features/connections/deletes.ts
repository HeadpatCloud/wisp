import type { Connection, Host, SavedLogin } from '@/bindings'
import { connectionLabel } from '@/lib/hosts'
import { confirm } from '@/stores/confirmStore'
import { useHostStore } from '@/stores/hostStore'
import { notify } from '@/stores/noticeStore'

const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`

function joinAnd(items: string[]): string {
  if (items.length <= 1) return items.join('')
  return `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`
}

const tunnelsOf = (c: Connection) => (c.kind.type === 'ssh' ? (c.kind.tunnels ?? []).length : 0)

// Connections elsewhere whose jump host is one of `ids`; deleting leaves their reference dangling.
function jumpWarning(hosts: Host[], logins: SavedLogin[], ids: Set<string>): string {
  const users = hosts.flatMap((h) =>
    h.connections
      .filter((c) => c.kind.type === 'ssh' && !ids.has(c.id) && ids.has(c.kind.jumpHostId ?? ''))
      .map((c) => `${h.name} · ${connectionLabel(h, c, logins)}`),
  )
  if (users.length === 0) return ''
  return users.length === 1
    ? ` ${users[0]} uses it as its jump host and stops connecting until you pick another.`
    : ` ${joinAnd(users)} use it as their jump host and stop connecting until you pick another.`
}

export function hostDeleteText(host: Host, hosts: Host[], logins: SavedLogin[]) {
  const n = host.connections.length
  const m = host.connections.reduce((sum, c) => sum + tunnelsOf(c), 0)
  const what =
    m > 0 ? `${count(n, 'connection')} and ${count(m, 'tunnel')}` : count(n, 'connection')
  const first = n === 0 ? '' : `Its ${what} ${n === 1 && m === 0 ? 'goes' : 'go'} with it. `
  return {
    title: `Delete ${host.name}?`,
    body: `${first}Saved logins and keys stay in Keys & passwords.${jumpWarning(hosts, logins, new Set(host.connections.map((c) => c.id)))}`,
  }
}

export function connectionDeleteText(
  host: Host,
  c: Connection,
  hosts: Host[],
  logins: SavedLogin[],
) {
  return {
    title: `Delete ${connectionLabel(host, c, logins)}?`,
    body: `It goes when you save.${tunnelsOf(c) > 0 ? ' Its tunnels go with it.' : ''}${jumpWarning(hosts, logins, new Set([c.id]))}`,
  }
}

export function confirmDeleteHost(host: Host): Promise<boolean> {
  const { hosts, logins } = useHostStore.getState()
  return confirm({
    ...hostDeleteText(host, hosts, logins),
    confirmLabel: 'Delete',
    destructive: true,
  })
}

// Undo puts the item back; the notice going any other way purges it with its secrets.
export function undoNotice(title: string, pendingId: string) {
  const store = useHostStore.getState()
  notify({
    title,
    actions: [
      {
        label: 'Undo',
        run: () => {
          // A failed Undo (its host deleted after it, say) offers it again, so it isn't purged.
          store.undoDelete(pendingId).catch((e: unknown) => {
            notify({
              title: "Couldn't undo",
              body: e instanceof Error ? e.message : String(e),
              tone: 'error',
            })
            undoNotice(title, pendingId)
          })
        },
      },
    ],
    onClose: () => {
      store.purgeDeletes([pendingId]).catch(console.error)
    },
  })
}

export async function deleteHostWithUndo(host: Host) {
  const pending = await useHostStore.getState().deleteHost(host.id)
  undoNotice(`${host.name} deleted`, pending)
}
