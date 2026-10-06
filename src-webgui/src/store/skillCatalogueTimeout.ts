import type { StoreGet, StoreSet } from './api'

// Catalogue discovery is request-driven. A lost or disconnected acknowledgement
// must not leave Rescan disabled forever; do not infer that the scan failed or
// silently retry a request whose result might still arrive later.
export const SKILL_CATALOGUE_WAIT_MS = 12_000

export function waitForSkillCatalogue(set: StoreSet, get: StoreGet, requestId: string, sessionEpoch: number): void {
  setTimeout(() => {
    const state = get()
    if (state.skillSessionEpoch !== sessionEpoch || state.skillRequestId !== requestId || !state.skillsLoading) return
    set({
      skillsLoading: false,
      skillsUnconfirmed: 'Skill catalogue response not confirmed. The list may be stale; try Rescan skill locations again.',
    })
  }, SKILL_CATALOGUE_WAIT_MS)
}
