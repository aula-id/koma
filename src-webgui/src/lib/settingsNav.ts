import { useKoma } from '../store/koma'

export type SettingsSectionId = 'about'

const SECTION_EVENT = 'koma-settings-section'
let pending: SettingsSectionId | null = null

/** Open the Settings tab and bring one of its sections into view. */
export function openSettingsSection(id: SettingsSectionId) {
  pending = id
  useKoma.getState().openSettingsTab()
  window.dispatchEvent(new CustomEvent(SECTION_EVENT))
}

/** About Koma lives in Settings; every other entry point calls this. */
export function showAbout() {
  openSettingsSection('about')
}

/** The section requested before Settings was mounted or visible, consumed once. */
export function takePendingSection(): SettingsSectionId | null {
  const id = pending
  pending = null
  return id
}

export function onSettingsSectionRequest(listener: () => void): () => void {
  window.addEventListener(SECTION_EVENT, listener)
  return () => window.removeEventListener(SECTION_EVENT, listener)
}
