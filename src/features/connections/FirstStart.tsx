import { Check, Download, SquareTerminal } from 'lucide-react'
import { useState } from 'react'
import type { Host } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { describeAddress, parseAddress } from '@/lib/address'
import { useHostStore } from '@/stores/hostStore'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { ConnectionButton } from '../hosts/ConnectionButton'
import { importFromFile } from '../transfer/importFromFile'
import { connectOnce, prefillOf } from './connectOnce'

const FORMS = [
  'host:2222',
  'sftp://host',
  'ftp://host',
  'rdp://host',
  'vnc://host',
  's3://endpoint/bucket',
]

export function FirstStart({ pc }: { pc: Host | null }) {
  const [text, setText] = useState('')
  const parsed = parseAddress(text)
  const logins = useHostStore((s) => s.logins)
  const shells = useHostStore((s) => s.shells)
  const openNewConnection = useHostsUiStore((s) => s.openNewConnection)
  const openPage = useHostsUiStore((s) => s.openPage)

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-8 py-8">
      <section className="flex flex-col gap-3">
        <h1 className="font-bold text-3xl tracking-tight">Add your first host</h1>
        <p className="text-muted-foreground">
          Paste an address. Wisp works out the type from what you type, and you can add more ways to
          connect to the same host later.
        </p>
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            if (parsed) connectOnce(parsed)
          }}
        >
          <label htmlFor="first-address" className="sr-only">
            Address
          </label>
          <Input
            id="first-address"
            value={text}
            placeholder="admin@nas.local"
            onChange={(e) => setText(e.target.value)}
            className="h-12 min-w-64 flex-1 font-mono text-base"
          />
          <Button type="submit" size="lg" disabled={!parsed}>
            Connect
          </Button>
          <Button
            type="button"
            size="lg"
            variant="outline"
            onClick={() => openNewConnection(parsed ? prefillOf(parsed) : {})}
          >
            Save as host…
          </Button>
        </form>
        {parsed ? (
          <p className="flex items-center gap-2 text-primary text-sm">
            <Check className="size-4" aria-hidden />
            {describeAddress(parsed)}
          </p>
        ) : (
          text.trim() && (
            <p className="text-destructive text-sm">Wisp can't read that as an address.</p>
          )
        )}
        <p className="text-muted-foreground text-sm">
          Also understood:{' '}
          {FORMS.map((f, i) => (
            <span key={f}>
              <code className="font-mono">{f}</code>
              {i < FORMS.length - 1 && ', '}
            </span>
          ))}
        </p>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="font-semibold text-muted-foreground text-sm uppercase tracking-wide">
          Or bring your hosts along
        </h2>
        <div className="grid gap-3 sm:grid-cols-2">
          <button
            type="button"
            onClick={() => openPage('sshImport')}
            className="flex items-center gap-3 rounded-xl border border-border bg-card p-4 text-left hover:bg-muted"
          >
            <SquareTerminal className="size-5 shrink-0 text-muted-foreground" aria-hidden />
            <span className="flex flex-col">
              <span className="font-semibold">Import from ~/.ssh/config</span>
              <span className="text-muted-foreground text-sm">Pick which hosts to bring in</span>
            </span>
          </button>
          <button
            type="button"
            onClick={() => void importFromFile()}
            className="flex items-center gap-3 rounded-xl border border-border bg-card p-4 text-left hover:bg-muted"
          >
            <Download className="size-5 shrink-0 text-muted-foreground" aria-hidden />
            <span className="flex flex-col">
              <span className="font-semibold">Import a Wisp export</span>
              <span className="text-muted-foreground text-sm">
                From another computer, password protected
              </span>
            </span>
          </button>
        </div>
      </section>

      {pc && pc.connections.length > 0 && (
        <section className="flex flex-col gap-3">
          <h2 className="font-semibold text-muted-foreground text-sm uppercase tracking-wide">
            This computer
          </h2>
          <div className="flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card p-4">
            <span className="flex-1 text-muted-foreground">Open a local shell</span>
            {pc.connections.map((c) => (
              <ConnectionButton
                key={c.id}
                host={pc}
                connection={c}
                logins={logins}
                shells={shells}
              />
            ))}
          </div>
        </section>
      )}
    </div>
  )
}
