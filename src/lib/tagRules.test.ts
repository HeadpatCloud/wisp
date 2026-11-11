import { expect, test, vi } from 'vitest'
import type { Host, Tag } from '@/bindings'
import { guardedPaste, rulesFor } from './tagRules'

const tag = (id: string, patch: Partial<Tag> = {}): Tag => ({
  id,
  name: id.toUpperCase(),
  color: 'grey',
  showColor: false,
  askBeforeClose: false,
  askBeforePaste: false,
  terminalColors: null,
  ...patch,
})
const host = (tagIds: string[]): Host =>
  ({
    id: 'h',
    name: 'web',
    address: 'web.lan',
    folderId: null,
    color: null,
    order: 0,
    connections: [],
    tagIds,
  }) as Host

test('tag_rules_combine_in_list_order', () => {
  // The Tags list order decides, not the order on the host.
  const tags = [
    tag('prod', { color: 'red', showColor: true, askBeforeClose: true }),
    tag('db', { color: 'blue', showColor: true, askBeforePaste: true, terminalColors: 'nord' }),
    tag('lab', { terminalColors: 'dracula' }),
  ]
  expect(rulesFor(host(['lab', 'db', 'prod']), tags)).toEqual({
    color: 'red',
    askBeforeClose: true,
    askBeforePaste: true,
    terminalColors: 'nord',
    closeTag: 'PROD',
  })
  // A tag that doesn't show its colour gives none.
  expect(rulesFor(host(['lab']), tags)).toMatchObject({ color: null, terminalColors: 'dracula' })
  expect(rulesFor(host([]), tags)).toEqual({
    color: null,
    askBeforeClose: false,
    askBeforePaste: false,
    terminalColors: null,
    closeTag: null,
  })
  expect(rulesFor(undefined, tags).askBeforeClose).toBe(false)
})

test('a_multi_line_paste_waits_for_paste', async () => {
  const paste = vi.fn()
  let answer: (ok: boolean) => void = () => {}
  const ask = vi.fn(
    () =>
      new Promise<boolean>((r) => {
        answer = r
      }),
  )

  // One line goes straight in, even with the rule on.
  await guardedPaste('ls -la', true, ask, paste)
  expect(paste).toHaveBeenCalledWith('ls -la')
  expect(ask).not.toHaveBeenCalled()

  paste.mockClear()
  const pending = guardedPaste('rm -rf build\nmake', true, ask, paste)
  await Promise.resolve()
  expect(ask).toHaveBeenCalledWith('rm -rf build\nmake')
  expect(paste).not.toHaveBeenCalled()
  answer(true)
  await pending
  expect(paste).toHaveBeenCalledWith('rm -rf build\nmake')

  // Cancel pastes nothing; without the rule nothing is asked.
  paste.mockClear()
  const cancelled = guardedPaste('a\r\nb', true, ask, paste)
  await Promise.resolve()
  answer(false)
  await cancelled
  expect(paste).not.toHaveBeenCalled()
  await guardedPaste('a\nb', false, ask, paste)
  expect(paste).toHaveBeenCalledWith('a\nb')
})
