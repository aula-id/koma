import { useKoma } from '../store/koma'
import { HELP_MANIFEST } from './helpKnowledge'
import { useGuide } from './tutorialTours'
/** Construct, don't serialize application state: no secrets, documents or chats. */
export function helpContext() {
  const s = useKoma.getState()
  const tab = s.ui.tabs.find(t => t.id === s.ui.activeTabId)
  const view = tab?.kind === 'codingFile' || tab?.kind === 'terminal' ? 'coding' : tab?.kind
  const platform = navigator.platform.toLowerCase()
  const guide = useGuide.getState()
  return {
    platform: platform.includes('mac') ? 'mac' : platform.includes('win') ? 'windows' : platform.includes('linux') ? 'linux' : 'unknown',
    attached: !!s.session.id,
    remote: !!s.remoteState.hostId,
    activeView: HELP_MANIFEST.navigation.includes(view ?? '') ? view : 'help',
    capabilities: HELP_MANIFEST.navigation.filter(id => document.querySelector(`[data-tour-view="${id}"]`) || ['help', 'settings', 'notifications', 'sessions'].includes(id)),
    hasProviders: s.config.providers.length > 0,
    hasModels: s.config.models.length > 0,
    hasMcp: s.config.mcp.length > 0,
    hasSearchConfig: !!s.webSearchValues?.status,
    hasWorkspace: !!s.coding.activeRoot,
    guide: guide.id,
    guideStep: guide.step,
  }
}
