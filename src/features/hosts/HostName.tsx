import type { Host } from '@/bindings'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { InlineName } from './InlineName'
import { openConnection } from './openConnection'

// A click selects the host; Enter or a double click opens its main connection.
export function HostName({ host }: { host: Host }) {
  const select = useHostsUiStore((s) => s.select)
  const renaming = useHostsUiStore((s) => s.renamingHostId === host.id)
  const setRenaming = useHostsUiStore((s) => s.setRenaming)
  const saveHost = useHostStore((s) => s.saveHost)
  const main = host.connections[0]
  const open = () => {
    if (main) openConnection(host, main)
  }
  if (renaming) {
    return (
      <InlineName
        initial={host.name}
        label="Host name"
        onSave={(name) => {
          setRenaming(null)
          saveHost({ ...host, name }).catch(console.error)
        }}
        onCancel={() => setRenaming(null)}
      />
    )
  }
  return (
    <button
      type="button"
      data-host-name={host.id}
      onClick={() => select(host.id)}
      onDoubleClick={open}
      onKeyDown={(e) => {
        if (e.key !== 'Enter') return
        e.preventDefault()
        open()
      }}
      className="truncate text-left hover:underline"
    >
      {host.name}
    </button>
  )
}
