import type { StoreGet, StoreSet } from '../api'
import { DEFAULT_GROUP, normalizeGroups } from '../editorGroups'
import { waitForSkillCatalogue } from '../skillCatalogueTimeout'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

let skillRequestSeq = 0
let skillCreateTabSeq = 0
function mintSkillRequestId(prefix = 'skills'): string {
  skillRequestSeq += 1
  return `${prefix}-${skillRequestSeq}`
}

export function skillActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'newSessionPreservingTabs' | 'refreshSkills' | 'registerSkillDelete' | 'setSkillQuery' | 'setSkillFilter' | 'setSkillSelection' | 'openSkillTab' | 'openUploadSkillTab' | 'requestSkillDetail' | 'readSkillFile'> {
  return {
  newSessionPreservingTabs: () => {
    set((s) => {
      const ui = normalizeGroups(s.ui)
      const chatGroup = ui.tabGroup.chat ?? DEFAULT_GROUP
      return {
        ui: {
          ...ui,
          preserveTabsOnNextSession: true,
          preservedTabsTargetSession: null,
          preservedTabLayout: {
            tabs: ui.tabs.map((tab) => ({ ...tab })),
            activeTabId: 'chat',
            groups: [...ui.groups],
            tabGroup: { ...ui.tabGroup },
            groupActive: { ...ui.groupActive, [chatGroup]: 'chat' },
            activeGroupId: chatGroup,
            splitDir: ui.splitDir,
            groupSizes: { ...ui.groupSizes },
            splitTree: ui.splitTree,
            groupSplitDir: { ...ui.groupSplitDir },
          },
        },
      }
    })
    get().req({ r: 'NewSession' })
  },
  refreshSkills: () => {
    const requestId = mintSkillRequestId('catalogue')
    const sessionEpoch = get().skillSessionEpoch
    const workspace = get().skillFilter === 'project' ? (get().coding.activeRoot ?? '') : ''
    set({ skillsLoading: true, skillsError: null, skillsUnconfirmed: null, skillRequestId: requestId })
    get().req({ r: 'GetSkills', requestId, sessionEpoch, workspace })
    waitForSkillCatalogue(set, get, requestId, sessionEpoch)
  },
  registerSkillDelete: (requestId, sessionEpoch, items) => set((s) => {
    if (sessionEpoch !== s.skillSessionEpoch) return s
    const recent = Object.entries(s.skillDeletePending).slice(-31)
    return {
      skillDeletePending: {
        ...Object.fromEntries(recent),
        [requestId]: { sessionEpoch, items: items.map(({ skillId, generation, name }) => ({ skillId, generation, name })) },
      },
    }
  }),
  setSkillQuery: (skillQuery) => set({ skillQuery }),
  setSkillFilter: (skillFilter) => set({ skillFilter }),
  setSkillSelection: (skillSelection) => set({ skillSelection }),
  openUploadSkillTab: () => set((s) => {
    const existing = s.ui.tabs.some((tab) => tab.id === 'upload-skill')
    return {
      ui: {
        ...s.ui,
        tabs: existing ? s.ui.tabs : [...s.ui.tabs, { id: 'upload-skill', kind: 'uploadSkill' }],
        activeTabId: 'upload-skill',
      },
    }
  }),
  openSkillTab: (skillId, title = 'New skill') => {
    let tabId = ''
    set((s) => {
      const existing = s.ui.tabs.find((t) => t.kind === 'skill' && t.skillId === skillId)
      if (existing && skillId !== null) {
        tabId = existing.id
        return { ui: { ...s.ui, activeTabId: existing.id } }
      }
      tabId = skillId === null ? `skill-new-${++skillCreateTabSeq}` : `skill:${skillId}`
      const tabs: Tab[] = [...s.ui.tabs, { id: tabId, kind: 'skill', skillId, title }]
      return { ui: { ...s.ui, tabs, activeTabId: tabId } }
    })
    if (skillId !== null) {
      const entry = get().skills.find((skill) => skill.skillId === skillId)
      if (entry) get().requestSkillDetail(tabId, skillId, entry.generation)
    }
  },
  requestSkillDetail: (tabId, skillId, generation) => {
    const requestId = mintSkillRequestId('detail')
    const sessionEpoch = get().skillSessionEpoch
    set((s) => ({
      skillDetailPending: { ...s.skillDetailPending, [tabId]: requestId },
      skillDetailErrors: { ...s.skillDetailErrors, [tabId]: '' },
    }))
    get().req({ r: 'GetSkillDetail', requestId, sessionEpoch, tabId, skillId, generation })
  },
  readSkillFile: (tabId, skillId, generation, path) => {
    const requestId = mintSkillRequestId('file')
    set((s) => ({
      skillDetailPending: { ...s.skillDetailPending, [tabId]: requestId },
      skillDetailErrors: { ...s.skillDetailErrors, [tabId]: '' },
    }))
    get().req({
      r: 'ReadSkillFile',
      requestId,
      sessionEpoch: get().skillSessionEpoch,
      tabId,
      skillId,
      generation,
      path,
    })
  },
  }
}
