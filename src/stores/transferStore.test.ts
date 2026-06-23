import { beforeEach, expect, test } from 'vitest'
import { useTransferStore } from './transferStore'

const t = (id: string) => ({
  id,
  dir: 'upload' as const,
  name: id,
  transferred: 0,
  total: 100,
  status: 'active' as const,
})

beforeEach(() => useTransferStore.setState({ transfers: [] }))

test('start adds a transfer', () => {
  useTransferStore.getState().start(t('a'))
  expect(useTransferStore.getState().transfers).toHaveLength(1)
})

test('progress updates one transfer', () => {
  useTransferStore.getState().start(t('a'))
  useTransferStore.getState().progress('a', 50, 100)
  expect(useTransferStore.getState().transfers[0].transferred).toBe(50)
})

test('remove drops only the targeted transfer', () => {
  const s = useTransferStore.getState()
  s.start(t('a'))
  s.start(t('b'))
  s.remove('a')
  expect(useTransferStore.getState().transfers.map((x) => x.id)).toEqual(['b'])
})

test('finish sets status; clearDone drops finished', () => {
  const s = useTransferStore.getState()
  s.start(t('a'))
  s.start(t('b'))
  s.finish('a')
  expect(useTransferStore.getState().transfers.find((x) => x.id === 'a')?.status).toBe('done')
  s.clearDone()
  expect(useTransferStore.getState().transfers.map((x) => x.id)).toEqual(['b'])
})
