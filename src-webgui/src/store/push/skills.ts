import type { StoreGet, StoreSet } from '../api'
import type { PushEnvelope } from '../types/envelope'
import { waitForSkillCatalogue } from '../skillCatalogueTimeout'

export function pushSkills(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'SkillValues': {
        if (env.sessionEpoch !== get().skillSessionEpoch) return true
        if (get().skillRequestId && env.requestId !== get().skillRequestId) return true
        const op = get().skillOpResults[env.requestId]
        const renamed = op && (op.operation === 'update' || op.operation === 'update-reload') && op.outcomes[0]?.status === 'success'
          ? env.skills.find((skill) => skill.name === op.outcomes[0]?.name)
          : undefined
        const currentTabs = get().ui.tabs
        let retarget: { tabId: string; skillId: string; generation: string } | null = null
        const nextTabs = renamed && op
          ? currentTabs.map((tab) => {
              if (tab.id !== op.tabId || tab.kind !== 'skill' || !tab.skillId || tab.skillId === renamed.skillId) return tab
              if (env.skills.some((skill) => skill.skillId === tab.skillId)) return tab
              retarget = { tabId: tab.id, skillId: renamed.skillId, generation: renamed.generation }
              return { ...tab, skillId: renamed.skillId, title: renamed.name }
            })
          : currentTabs
        set((state) => ({
          skills: env.skills,
          loadedSkillNames: env.loadedSkillNames,
          skillsLoading: false,
          skillsError: env.error,
          skillsUnconfirmed: null,
          // Retain the accepted id so an older same-epoch reply arriving later
          // cannot become acceptable merely because this request completed.
          skillRequestId: env.requestId,
          ...(retarget ? { ui: { ...state.ui, tabs: nextTabs } } : {}),
        }))
        if (retarget) get().requestSkillDetail(retarget.tabId, retarget.skillId, retarget.generation)
        return true
      }
      case 'SkillDetailValues': {
        if (env.sessionEpoch !== get().skillSessionEpoch) return true
        if (get().skillDetailPending[env.tabId] !== env.requestId) return true
        set((s) => {
          const pending = { ...s.skillDetailPending }
          delete pending[env.tabId]
          const errors = { ...s.skillDetailErrors }
          if (env.error) errors[env.tabId] = env.error
          else delete errors[env.tabId]
          const details = env.detail ? { ...s.skillDetails, [env.tabId]: env.detail } : s.skillDetails
          const files = env.filePath !== null
            ? {
                ...s.skillFiles,
                [`${env.tabId}:${env.filePath}`]: {
                  content: env.fileContent ?? '',
                  error: env.error,
                },
              }
            : s.skillFiles
          return {
            skillDetailPending: pending,
            skillDetailErrors: errors,
            skillDetails: details,
            skillFiles: files,
          }
        })
        return true
      }
      case 'SkillOp': {
        if (env.sessionEpoch !== get().skillSessionEpoch) return true
        const pendingDelete = get().skillDeletePending[env.requestId]
        // Both attached and detached backends return exactly one outcome per
        // requested item, in order. Names alone are not unique source IDs.
        const deletedIds = pendingDelete?.sessionEpoch === env.sessionEpoch &&
          env.operation === 'delete' && env.tabId === 'delete-dialog' &&
          env.outcomes.length === pendingDelete.items.length &&
          env.outcomes.every((item, index) => item.name === pendingDelete.items[index].name)
          ? new Set(pendingDelete.items.filter((_, index) => env.outcomes[index].status === 'success').map((item) => item.skillId))
          : new Set<string>()
        const changesFilesystem = ['create', 'install-zip', 'update', 'update-reload', 'duplicate', 'delete', 'set-roots'].includes(env.operation)
        set((s) => {
          const result = { requestId: env.requestId, operation: env.operation, tabId: env.tabId, outcomes: env.outcomes }
          // Keep a small request-correlated history so a later bulk operation
          // cannot replace an editor's own Save acknowledgment.
          const recent = Object.entries(s.skillOpResults).slice(-31)
          const { [env.requestId]: _resolvedDelete, ...remainingDeletes } = s.skillDeletePending
          return {
            skillDeletePending: remainingDeletes,
            skillOutcomes: env.outcomes,
            skillLastOp: result,
            skillOpResults: { ...Object.fromEntries(recent), [env.requestId]: result },
            loadedSkillNames: env.loadedSkillNames,
            // Filesystem mutations are followed by SkillValues with this id;
            // Load/Unload/Reload only replace session context in SkillOp.
            ...(changesFilesystem ? { skillRequestId: env.requestId, skillsLoading: true, skillsUnconfirmed: null } : {}),
          }
        })
        if (changesFilesystem) waitForSkillCatalogue(set, get, env.requestId, env.sessionEpoch)
        // Close only the editor tabs for confirmed deleted source identities.
        // Do it in the push handler, not the dialog effect: a SkillValues reply
        // can unmount that dialog before React gets to run its effect.
        if (deletedIds.size) {
          for (const tab of get().ui.tabs) {
            if (tab.kind !== 'skill' || !tab.skillId || !deletedIds.has(tab.skillId)) continue
            const currentDetail = get().skillDetails[tab.id]
            const requested = pendingDelete?.items.find((item) => item.skillId === tab.skillId)
            if (currentDetail && currentDetail.generation !== requested?.generation) continue
            get().closeTab(tab.id)
          }
        }
        return true
      }
    default: return false
  }
}
