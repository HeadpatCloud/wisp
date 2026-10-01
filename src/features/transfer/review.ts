import type { ImportReview, ItemDecision, ReviewItem } from '@/bindings'

export interface Decision {
  accept: boolean
  asNew: boolean
  fields: Record<string, boolean>
}

export type Decisions = Record<string, Decision>
export type Tri = 'all' | 'some' | 'none'

// A changed profile is decided field by field unless it's being added as a separate copy.
function byFields(item: ReviewItem, d: Decision): boolean {
  return item.status === 'conflict' && !d.asNew
}

export function initialDecisions(review: ImportReview): Decisions {
  return Object.fromEntries(
    review.items.map((item) => [
      item.key,
      {
        accept: true,
        asNew: false,
        fields: Object.fromEntries(item.fields.map((f) => [f.field, true])),
      },
    ]),
  )
}

export function itemState(item: ReviewItem, d: Decision): Tri {
  if (!byFields(item, d)) return d.accept ? 'all' : 'none'
  const on = item.fields.filter((f) => d.fields[f.field]).length
  if (on === item.fields.length) return 'all'
  return on === 0 ? 'none' : 'some'
}

export function setItem(decisions: Decisions, item: ReviewItem, on: boolean): Decisions {
  return {
    ...decisions,
    [item.key]: {
      ...decisions[item.key],
      accept: on,
      fields: Object.fromEntries(item.fields.map((f) => [f.field, on])),
    },
  }
}

export function setField(
  decisions: Decisions,
  item: ReviewItem,
  field: string,
  on: boolean,
): Decisions {
  const d = decisions[item.key]
  return { ...decisions, [item.key]: { ...d, fields: { ...d.fields, [field]: on } } }
}

export function setAsNew(decisions: Decisions, item: ReviewItem, asNew: boolean): Decisions {
  return { ...decisions, [item.key]: { ...decisions[item.key], asNew, accept: true } }
}

export function setAll(review: ImportReview, decisions: Decisions, on: boolean): Decisions {
  return review.items.reduce((acc, item) => setItem(acc, item, on), decisions)
}

export function toPayload(review: ImportReview, decisions: Decisions): ItemDecision[] {
  return review.items.map((item) => {
    const d = decisions[item.key]
    return {
      key: item.key,
      accept: itemState(item, d) !== 'none',
      asNew: d.asNew,
      fields: byFields(item, d)
        ? item.fields.filter((f) => d.fields[f.field]).map((f) => f.field)
        : [],
    }
  })
}
