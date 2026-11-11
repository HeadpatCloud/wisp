import { expect, test } from 'vitest'
import type { ImportReview } from '@/bindings'
import {
  initialDecisions,
  itemState,
  setAll,
  setAsNew,
  setField,
  setItem,
  toggleItem,
  toPayload,
} from './review'

const review: ImportReview = {
  reviewId: 'r1',
  unchanged: 3,
  items: [
    {
      key: 'ssh:a',
      kind: 'connection',
      parent: null,
      name: 'web',
      status: 'conflict',
      matched: { id: 'pc', name: 'web' },
      fields: [
        { field: 'port', label: 'Port', local: '22', incoming: '2222' },
        { field: 'host', label: 'Host', local: 'a', incoming: 'b' },
      ],
      notes: [],
      notesAsNew: [],
    },
    {
      key: 'ssh:n',
      kind: 'connection',
      parent: null,
      name: 'new',
      status: 'new',
      matched: null,
      fields: [],
      notes: [],
      notesAsNew: [],
    },
  ],
}
const [conflict, fresh] = review.items

test('everything starts accepted', () => {
  const d = initialDecisions(review)
  expect(itemState(conflict, d['ssh:a'])).toBe('all')
  expect(itemState(fresh, d['ssh:n'])).toBe('all')
})

test('declining one field makes the profile partial, all fields makes it declined', () => {
  let d = setField(initialDecisions(review), conflict, 'port', false)
  expect(itemState(conflict, d['ssh:a'])).toBe('some')
  d = setField(d, conflict, 'host', false)
  expect(itemState(conflict, d['ssh:a'])).toBe('none')
  expect(toPayload(review, d)[0]).toEqual({ key: 'ssh:a', accept: false, asNew: false, fields: [] })
})

test('profile toggle sets every field', () => {
  const d = setItem(initialDecisions(review), conflict, false)
  expect(itemState(conflict, d['ssh:a'])).toBe('none')
  expect(itemState(conflict, setItem(d, conflict, true)['ssh:a'])).toBe('all')
})

test('accept all and decline all', () => {
  const off = setAll(review, initialDecisions(review), false)
  expect(toPayload(review, off).every((p) => !p.accept)).toBe(true)
  const on = setAll(review, off, true)
  expect(toPayload(review, on).every((p) => p.accept)).toBe(true)
})

test('add as new sends the whole item without fields', () => {
  const d = setAsNew(setField(initialDecisions(review), conflict, 'port', false), conflict, true)
  expect(toPayload(review, d)[0]).toEqual({ key: 'ssh:a', accept: true, asNew: true, fields: [] })
})

test('payload lists accepted fields only', () => {
  const d = setField(initialDecisions(review), conflict, 'host', false)
  expect(toPayload(review, d)).toEqual([
    { key: 'ssh:a', accept: true, asNew: false, fields: ['port'] },
    { key: 'ssh:n', accept: true, asNew: false, fields: [] },
  ])
})

const hostReview: ImportReview = {
  reviewId: 'r2',
  unchanged: 0,
  items: [
    { ...fresh, key: 'host:h', kind: 'host', name: 'nas' },
    { ...fresh, key: 'conn:a', kind: 'connection', name: 'nas · SSH · admin', parent: 'host:h' },
    { ...fresh, key: 'conn:b', kind: 'connection', name: 'nas · Files', parent: 'host:h' },
  ],
}

test('declining a new host declines its connections', () => {
  const [h, a] = hostReview.items
  const off = toggleItem(hostReview, initialDecisions(hostReview), h, false)
  expect(toPayload(hostReview, off).map((p) => p.accept)).toEqual([false, false, false])
  const back = toggleItem(hostReview, off, a, true)
  expect(toPayload(hostReview, back).map((p) => p.accept)).toEqual([true, true, false])
})
