import { useCallback, useEffect, useId, useRef, useState } from 'react'
import type { ApplySummary, ImportReview, ItemKind, ItemProblem, ReviewItem } from '@/bindings'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PageShell } from '@/components/ui/page-shell'
import {
  applyImport,
  discardImport,
  type ReadResult,
  readBundle,
  validateImport,
} from '@/lib/transfer'
import { cn } from '@/lib/utils'
import { useProfileStore } from '@/stores/profileStore'
import { useS3ProfileStore } from '@/stores/s3ProfileStore'
import { useSessionStore } from '@/stores/sessionStore'
import { useSftpProfileStore } from '@/stores/sftpProfileStore'
import {
  type Decisions,
  initialDecisions,
  itemState,
  setAll,
  setAsNew,
  setField,
  setItem,
  toPayload,
} from './review'
import { TriCheckbox } from './TriCheckbox'

const KIND: Record<ItemKind, string> = { group: 'Group', ssh: 'SSH', sftp: 'SFTP', s3: 'S3' }

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

export function ImportReviewPage({ tabId, path }: { tabId: string; path: string }) {
  const removeTab = useSessionStore((s) => s.removeTab)
  const reloadProfiles = useProfileStore((s) => s.load)
  const reloadSftp = useSftpProfileStore((s) => s.load)
  const reloadS3 = useS3ProfileStore((s) => s.load)
  const [stage, setStage] = useState<'loading' | 'password' | 'review' | 'done'>('loading')
  const [password, setPassword] = useState('')
  const [wrongPassword, setWrongPassword] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [review, setReview] = useState<ImportReview | null>(null)
  const [decisions, setDecisions] = useState<Decisions>({})
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [problems, setProblems] = useState<ItemProblem[]>([])
  const [busy, setBusy] = useState(false)
  const [summary, setSummary] = useState<ApplySummary | null>(null)
  const reviewIdRef = useRef<string | null>(null)
  const runRef = useRef(0)
  const failedRef = useRef<Decisions | null>(null)
  // Several import tabs can be mounted at once, so fixed ids would collide.
  const formId = useId()
  const passwordId = useId()

  // StrictMode mounts twice; a read that lands after its run ended still holds a decrypted
  // bundle in the backend, so it is discarded instead of shown.
  const handle = useCallback((res: ReadResult, run: number) => {
    if (run !== runRef.current) {
      if (res.kind === 'review') discardImport(res.review.reviewId).catch(console.error)
      return
    }
    if (res.kind === 'review') {
      reviewIdRef.current = res.review.reviewId
      setReview(res.review)
      setDecisions(initialDecisions(res.review))
      setSelectedKey(
        (res.review.items.find((i) => i.status === 'conflict') ?? res.review.items[0])?.key ?? null,
      )
      setPassword('')
      setStage('review')
      return
    }
    setWrongPassword(res.kind === 'wrongPassword')
    setStage('password')
  }, [])

  useEffect(() => {
    const run = ++runRef.current
    readBundle(path, null)
      .then((res) => handle(res, run))
      .catch((e) => setError(errorText(e)))
    return () => {
      runRef.current++
      if (reviewIdRef.current) {
        discardImport(reviewIdRef.current).catch(console.error)
        reviewIdRef.current = null
      }
    }
  }, [path, handle])

  useEffect(() => {
    if (!review || busy || stage !== 'review') return
    let active = true
    const timer = setTimeout(() => {
      validateImport(review.reviewId, toPayload(review, decisions))
        .then((found) => {
          if (!active) return
          setProblems(found)
          // The re-check that follows a failed apply must not wipe that apply's message.
          if (decisions !== failedRef.current) setError(null)
        })
        .catch((e) => {
          if (active) setError(errorText(e))
        })
    }, 200)
    return () => {
      active = false
      clearTimeout(timer)
    }
  }, [review, decisions, busy, stage])

  function submitPassword() {
    if (busy || !password) return
    const run = runRef.current
    setBusy(true)
    setError(null)
    setWrongPassword(false)
    readBundle(path, password)
      .then((res) => handle(res, run))
      .catch((e) => setError(errorText(e)))
      .finally(() => setBusy(false))
  }

  async function doApply() {
    if (!review) return
    setBusy(true)
    setError(null)
    try {
      const result = await applyImport(review.reviewId, toPayload(review, decisions))
      reviewIdRef.current = null
      setSummary(result)
      setStage('done')
      await Promise.all([reloadProfiles(), reloadSftp(), reloadS3()])
    } catch (e) {
      failedRef.current = decisions
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const close = (
    <Button type="button" onClick={() => removeTab(tabId)}>
      Close
    </Button>
  )

  if (stage === 'done' && summary) {
    return (
      <PageShell title="Import profiles" footer={close}>
        <div className="space-y-2 text-sm">
          <p>
            Imported: {summary.added} added, {summary.updated} updated.
          </p>
          {error && (
            <p className="text-destructive">The profile list could not be refreshed: {error}</p>
          )}
        </div>
      </PageShell>
    )
  }

  if (stage === 'password') {
    return (
      <PageShell
        title="Import profiles"
        footer={
          <>
            <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
              Cancel
            </Button>
            <Button type="submit" form={formId} disabled={!password || busy}>
              Open
            </Button>
          </>
        }
      >
        <form
          id={formId}
          className="max-w-sm space-y-2 text-sm"
          onSubmit={(e) => {
            e.preventDefault()
            submitPassword()
          }}
        >
          <p>This export is encrypted.</p>
          <div className="space-y-1">
            <Label htmlFor={passwordId}>Export password</Label>
            <Input
              id={passwordId}
              type="password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          {wrongPassword && <p className="text-destructive">Wrong password.</p>}
          {error && <p className="text-destructive">{error}</p>}
        </form>
      </PageShell>
    )
  }

  if (!review) {
    return (
      <PageShell title="Import profiles" footer={close}>
        {error ? (
          <p className="text-destructive text-sm">{error}</p>
        ) : (
          <p className="text-muted-foreground text-sm">Reading export...</p>
        )}
      </PageShell>
    )
  }

  const changed = review.items.filter((i) => i.status === 'conflict')
  const fresh = review.items.filter((i) => i.status === 'new')
  const selected = review.items.find((i) => i.key === selectedKey) ?? null
  const notes = selected
    ? [...new Set(decisions[selected.key].asNew ? selected.notesAsNew : selected.notes)]
    : []
  const problemsFor = (key: string) => [
    ...new Set(problems.filter((p) => p.key === key).map((p) => p.message)),
  ]
  const nothingAccepted = review.items.every((i) => itemState(i, decisions[i.key]) === 'none')

  const row = (item: ReviewItem) => (
    <li key={item.key}>
      <div
        className={cn(
          'flex items-center gap-2 rounded px-2 py-1',
          item.key === selectedKey ? 'bg-muted' : 'hover:bg-muted',
        )}
      >
        <TriCheckbox
          state={itemState(item, decisions[item.key])}
          label={`Import ${item.name}`}
          onChange={(on) => setDecisions((d) => setItem(d, item, on))}
        />
        <button
          type="button"
          className="min-w-0 flex-1 truncate text-left"
          onClick={() => setSelectedKey(item.key)}
        >
          {item.name}
          {item.matched && item.matched.name !== item.name && (
            <span className="text-muted-foreground"> → {item.matched.name}</span>
          )}
        </button>
        <span className="text-muted-foreground text-xs">{KIND[item.kind]}</span>
        {problemsFor(item.key).length > 0 && <span className="text-destructive text-xs">!</span>}
      </div>
    </li>
  )

  return (
    <PageShell
      title="Import profiles"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => removeTab(tabId)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={busy || problems.length > 0 || nothingAccepted}
            onClick={doApply}
          >
            Apply
          </Button>
        </>
      }
    >
      <div className="flex h-full flex-col gap-3 text-sm">
        <div className="flex items-center gap-3">
          <span>
            {fresh.length} new · {changed.length} with changes · {review.unchanged} unchanged
          </span>
          <div className="ml-auto flex gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDecisions((d) => setAll(review, d, true))}
            >
              Accept all
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDecisions((d) => setAll(review, d, false))}
            >
              Decline all
            </Button>
          </div>
        </div>
        {error && <p className="text-destructive">{error}</p>}
        {review.items.length === 0 ? (
          <p className="text-muted-foreground">Nothing to import; everything already matches.</p>
        ) : (
          <div className="flex min-h-0 flex-1 gap-4">
            <div className="w-72 shrink-0 space-y-3 overflow-y-auto border-border border-r pr-3">
              {changed.length > 0 && (
                <section>
                  <h3 className="mb-1 font-semibold">With changes</h3>
                  <ul className="space-y-0.5">{changed.map(row)}</ul>
                </section>
              )}
              {fresh.length > 0 && (
                <section>
                  <h3 className="mb-1 font-semibold">New</h3>
                  <ul className="space-y-0.5">{fresh.map(row)}</ul>
                </section>
              )}
            </div>
            {selected && (
              <div className="min-w-0 flex-1 space-y-3 overflow-y-auto">
                <div>
                  <h3 className="font-semibold">{selected.name}</h3>
                  <p className="text-muted-foreground">
                    {selected.matched
                      ? `Matches your ${KIND[selected.kind]} "${selected.matched.name}"`
                      : `New ${KIND[selected.kind]}`}
                  </p>
                </div>
                {selected.status === 'conflict' && (
                  <>
                    <label className="flex items-center gap-2">
                      <input
                        type="checkbox"
                        aria-label="Add as new instead"
                        checked={decisions[selected.key].asNew}
                        onChange={(e) =>
                          setDecisions((d) => setAsNew(d, selected, e.target.checked))
                        }
                      />
                      Add as new instead
                    </label>
                    {decisions[selected.key].asNew ? (
                      <p className="text-muted-foreground">
                        It will be added as a separate profile.
                      </p>
                    ) : (
                      <>
                        <div className="flex gap-2">
                          <Button
                            type="button"
                            variant="ghost"
                            onClick={() => setDecisions((d) => setItem(d, selected, true))}
                          >
                            Accept all fields
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            onClick={() => setDecisions((d) => setItem(d, selected, false))}
                          >
                            Decline all fields
                          </Button>
                        </div>
                        <table className="w-full">
                          <thead className="text-left text-muted-foreground">
                            <tr>
                              <th className="py-1 font-normal">Field</th>
                              <th className="py-1 font-normal">This machine</th>
                              <th className="py-1 font-normal">Incoming</th>
                              <th className="py-1 font-normal">Accept</th>
                            </tr>
                          </thead>
                          <tbody>
                            {selected.fields.map((f) => (
                              <tr key={f.field} className="border-border border-t align-top">
                                <td className="py-1 pr-2">{f.label}</td>
                                <td className="break-all py-1 pr-2">{f.local}</td>
                                <td className="break-all py-1 pr-2">{f.incoming}</td>
                                <td className="py-1">
                                  <input
                                    type="checkbox"
                                    aria-label={`Accept ${f.label}`}
                                    checked={decisions[selected.key].fields[f.field] ?? false}
                                    onChange={(e) =>
                                      setDecisions((d) =>
                                        setField(d, selected, f.field, e.target.checked),
                                      )
                                    }
                                  />
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </>
                    )}
                  </>
                )}
                {notes.length > 0 && (
                  <ul className="space-y-1 text-muted-foreground">
                    {notes.map((n) => (
                      <li key={n}>{n}</li>
                    ))}
                  </ul>
                )}
                {problemsFor(selected.key).map((m) => (
                  <p key={m} className="text-destructive">
                    {m}
                  </p>
                ))}
              </div>
            )}
          </div>
        )}
        {review.items
          .filter((i) => i.key !== selectedKey)
          .flatMap((i) =>
            problemsFor(i.key).map((m) => (
              <p key={`${i.key}-${m}`} className="text-destructive">
                {i.name}: {m}
              </p>
            )),
          )}
      </div>
    </PageShell>
  )
}
