import { open, save } from '@tauri-apps/plugin-dialog'
import {
  type ApplySummary,
  commands,
  type ExportOptions,
  type ExportSelection,
  type ExportSummary,
  type ImportReview,
  type ItemDecision,
  type ItemProblem,
} from '@/bindings'
import { unwrap } from '@/lib/ipc'

const filters = [{ name: 'JSON', extensions: ['json'] }]

export async function pickExportPath(): Promise<string | null> {
  return (await save({ defaultPath: 'wisp-profiles.json', filters })) ?? null
}

export async function pickImportPath(): Promise<string | null> {
  const path = await open({ filters })
  return typeof path === 'string' ? path : null
}

export async function exportBundle(
  selection: ExportSelection,
  options: ExportOptions,
  password: string | null,
  path: string,
): Promise<ExportSummary> {
  return unwrap(await commands.transferExport(selection, options, password, path))
}

export type ReadResult =
  | { kind: 'needsPassword' }
  | { kind: 'wrongPassword' }
  | { kind: 'review'; review: ImportReview }

export async function readBundle(path: string, password: string | null): Promise<ReadResult> {
  const res = await commands.transferRead(path, password)
  if (res.status === 'error' && res.error.kind === 'wrongPassphrase')
    return { kind: 'wrongPassword' }
  const out = unwrap(res)
  return out.kind === 'needsPassword'
    ? { kind: 'needsPassword' }
    : { kind: 'review', review: out.review }
}

export async function validateImport(
  reviewId: string,
  decisions: ItemDecision[],
): Promise<ItemProblem[]> {
  return unwrap(await commands.transferValidate(reviewId, decisions))
}

export async function applyImport(
  reviewId: string,
  decisions: ItemDecision[],
): Promise<ApplySummary> {
  return unwrap(await commands.transferApply(reviewId, decisions))
}

export async function discardImport(reviewId: string): Promise<void> {
  unwrap(await commands.transferDiscard(reviewId))
}
