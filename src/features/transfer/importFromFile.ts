import { pickImportPath } from '@/lib/transfer'
import { useHostsUiStore } from '@/stores/hostsUiStore'

export async function importFromFile() {
  const path = await pickImportPath()
  if (path) useHostsUiStore.getState().openPage('import', path)
}
