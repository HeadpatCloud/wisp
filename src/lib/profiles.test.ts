import { expect, test } from 'vitest'
import type { Group, Profile } from '@/bindings'
import {
  duplicateProfile,
  nextGroupOrder,
  nextProfileOrder,
  reorderGroups,
  reorderProfiles,
  wouldCycle,
} from './profiles'

const p = (id: string, jump: string | null): Profile => ({
  id,
  name: id,
  groupId: null,
  host: 'h',
  port: 22,
  username: 'u',
  authMethod: 'agent',
  keyPath: null,
  secretId: null,
  icon: { kind: 'builtin', name: 'server' },
  order: 0,
  jumpHostId: jump,
  tunnels: [],
})

test('detects a direct self jump', () => {
  expect(wouldCycle([], 'a', 'a')).toBe(true)
})

test('detects an indirect cycle a -> b -> a', () => {
  expect(wouldCycle([p('b', 'a')], 'a', 'b')).toBe(true)
})

test('allows a non-cyclic chain', () => {
  expect(wouldCycle([p('b', 'c'), p('c', null)], 'a', 'b')).toBe(false)
})

test('a new profile (no id) never cycles', () => {
  expect(wouldCycle([p('b', 'a')], null, 'b')).toBe(false)
})

const withOrder = (id: string, groupId: string | null, order: number): Profile => ({
  ...p(id, null),
  groupId,
  order,
})

const g = (id: string, order: number): Group => ({
  id,
  name: id,
  parentId: null,
  icon: { kind: 'builtin', name: 'cloud' },
  order,
})

test('nextProfileOrder is 0 for an empty group, else max+1 within that group', () => {
  expect(nextProfileOrder([], null)).toBe(0)
  const list = [withOrder('a', null, 0), withOrder('b', null, 3), withOrder('c', 'g1', 9)]
  expect(nextProfileOrder(list, null)).toBe(4)
  expect(nextProfileOrder(list, 'g1')).toBe(10)
})

test('nextGroupOrder is 0 when empty, else max+1', () => {
  expect(nextGroupOrder([])).toBe(0)
  expect(nextGroupOrder([g('a', 0), g('b', 5)])).toBe(6)
})

test('reorderProfiles moves a profile into a group and resequences orders', () => {
  const list = [withOrder('a', 'g1', 0), withOrder('b', 'g1', 1), withOrder('c', null, 0)]
  const changed = reorderProfiles(list, 'c', 'g1', 'b', false)
  const byId = Object.fromEntries(changed.map((x) => [x.id, x]))
  expect(byId.c.groupId).toBe('g1')
  expect(byId.c.order).toBe(1)
  expect(byId.b.order).toBe(2)
})

test('reorderProfiles placeAfter inserts after the target', () => {
  const list = [withOrder('a', null, 0), withOrder('b', null, 1)]
  const changed = reorderProfiles(list, 'a', null, 'b', true)
  const byId = Object.fromEntries(changed.map((x) => [x.id, x]))
  expect(byId.a.order).toBe(1)
  expect(byId.b.order).toBe(0)
})

test('reorderProfiles is a no-op when the position is unchanged', () => {
  const list = [withOrder('a', null, 0), withOrder('b', null, 1)]
  expect(reorderProfiles(list, 'a', null, 'b', false)).toEqual([])
})

test('reorderGroups resequences group order', () => {
  const changed = reorderGroups([g('a', 0), g('b', 1), g('c', 2)], 'c', 'a', false)
  const byId = Object.fromEntries(changed.map((x) => [x.id, x]))
  expect(byId.c.order).toBe(0)
  expect(byId.a.order).toBe(1)
  expect(byId.b.order).toBe(2)
})

test('duplicateProfile clones with a new id, "(copy)" name, no shared secret', () => {
  const src = {
    ...p('a', null),
    name: 'web',
    secretId: 'sec-1',
    tunnels: [
      {
        id: 't-1',
        kind: 'local' as const,
        bindHost: '127.0.0.1',
        bindPort: 8080,
        targetHost: 'x',
        targetPort: 80,
        autoStart: false,
      },
    ],
  }
  const copy = duplicateProfile(src)
  expect(copy.id).not.toBe(src.id)
  expect(copy.name).toBe('web (copy)')
  expect(copy.secretId).toBeNull()
  expect(copy.host).toBe(src.host)
  expect(copy.tunnels?.[0]?.id).not.toBe('t-1')
})
