import { create } from 'zustand'

// The tab being dragged in the tab bar, so the visible tab's content can offer its edges.
export const useTabDrag = create<{ tabId: string | null }>()(() => ({ tabId: null }))
