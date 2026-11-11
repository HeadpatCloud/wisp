import { create } from 'zustand'

export const NOTICE_MS = 8000

export interface NoticeAction {
  label: string
  run: () => void
}

export interface NoticeInput {
  title: string
  body?: string
  tone?: 'info' | 'error'
  actions?: NoticeAction[]
  sticky?: boolean
  // Runs when the notice goes by timeout or Dismiss, never when one of its actions is clicked.
  onClose?: () => void
}

export interface Notice extends NoticeInput {
  id: string
}

interface NoticeState {
  notices: Notice[]
  notify: (n: NoticeInput) => string
  dismiss: (id: string) => void
  act: (id: string, action: NoticeAction) => void
  update: (id: string, patch: Partial<NoticeInput>) => void
  hold: (id: string) => void
  release: (id: string) => void
}

const timers = new Map<
  string,
  { timer: ReturnType<typeof setTimeout> | null; left: number; since: number }
>()

function startTimer(id: string, ms: number) {
  timers.set(id, {
    timer: setTimeout(() => useNoticeStore.getState().dismiss(id), ms),
    left: ms,
    since: Date.now(),
  })
}

function remove(id: string): Notice | undefined {
  const t = timers.get(id)
  if (t?.timer) clearTimeout(t.timer)
  timers.delete(id)
  const notice = useNoticeStore.getState().notices.find((n) => n.id === id)
  useNoticeStore.setState((s) => ({ notices: s.notices.filter((n) => n.id !== id) }))
  return notice
}

export const useNoticeStore = create<NoticeState>()((set) => ({
  notices: [],
  notify: (n) => {
    const id = crypto.randomUUID()
    set((s) => ({ notices: [...s.notices, { ...n, id }] }))
    if (!n.sticky) startTimer(id, NOTICE_MS)
    return id
  },
  dismiss: (id) => remove(id)?.onClose?.(),
  act: (id, action) => {
    remove(id)
    action.run()
  },
  update: (id, patch) =>
    set((s) => ({ notices: s.notices.map((n) => (n.id === id ? { ...n, ...patch } : n)) })),
  hold: (id) => {
    const t = timers.get(id)
    if (!t?.timer) return
    clearTimeout(t.timer)
    timers.set(id, { timer: null, left: t.left - (Date.now() - t.since), since: Date.now() })
  },
  release: (id) => {
    const t = timers.get(id)
    if (!t || t.timer) return
    startTimer(id, t.left)
  },
}))

export const notify = (n: NoticeInput) => useNoticeStore.getState().notify(n)
