import { designTabId, serializeDesign, type DesignDoc } from '../../lib/design'
import type { StoreGet, StoreSet } from '../api'
import { baseName, fileKey, mintRequestId } from '../coding'
import { emptyDesignFile, emptyDesignFileUi, normalizeDesignSlice } from '../design'
import { normalizeGroups } from '../editorGroups'
import type { DesignFileUiState } from '../design'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

const DIRTY_CHECK_MS = 400
const dirtyChecks = new Map<string, ReturnType<typeof setTimeout>>()

export function designActions(set: StoreSet, get: StoreGet): Pick<
  KomaState,
  'openDesignTab' | 'saveDesign' | 'updateDesign' | 'createDesignFile' | 'setDesignPanelTab' | 'setDesignFileUi'
> {
  return {
    setDesignPanelTab: (id) => {
      set((s) => (s.design?.panelTabId === id ? s : { design: normalizeDesignSlice({ ...s.design, panelTabId: id }) }))
    },
    setDesignFileUi: (root, path, patch) => {
      const key = fileKey(root, path)
      set((s) => {
        const fileUi = s.design?.fileUi ?? {}
        const prev = fileUi[key] ?? emptyDesignFileUi()
        const next: DesignFileUiState = { ...prev, ...patch }
        if (
          prev.selection === next.selection &&
          prev.focusId === next.focusId &&
          prev.overrideTargetId === next.overrideTargetId &&
          prev.view === next.view
        ) {
          return s
        }
        return {
          design: normalizeDesignSlice({
            ...s.design,
            fileUi: { ...fileUi, [key]: next },
          }),
        }
      })
    },
    openDesignTab: (root, path) => {
      const id = designTabId(root, path)
      const key = fileKey(root, path)
      const existing = get().design?.docs?.[key]
      const readReq =
        existing && (existing.loading || existing.saving || existing.dirty || existing.savedText != null)
          ? null
          : mintRequestId()
      set((s) => {
        const base = normalizeGroups(s.ui)
        const exists = base.tabs.some((t) => t.id === id)
        const tabs: Tab[] = exists
          ? base.tabs
          : [...base.tabs, { id, kind: 'design', root, path, title: baseName(path) }]
        const ui = normalizeGroups({ ...base, tabs, activeTabId: id })
        if (!readReq) {
          return { ui, design: normalizeDesignSlice({ ...s.design, panelTabId: id }) }
        }
        const prev = s.design?.docs?.[key]
        return {
          ui,
          design: normalizeDesignSlice({
            ...s.design,
            panelTabId: id,
            docs: {
              ...s.design?.docs,
              [key]: emptyDesignFile({
                ...(prev ?? {}),
                loading: true,
                readReq,
                error: null,
              }),
            },
          }),
        }
      })
      if (readReq) get().req({ r: 'FileRead', root, path, requestId: readReq })
    },
    saveDesign: (root, path) => {
      const key = fileKey(root, path)
      const file = get().design?.docs?.[key]
      if (!file || file.loading || file.saving) return
      const content = serializeDesign(file.doc)
      const requestId = mintRequestId()
      set((s) => {
        const prev = s.design?.docs?.[key]
        if (!prev || prev.loading || prev.saving) return s
        return {
          design: normalizeDesignSlice({
            ...s.design,
            docs: {
              ...s.design?.docs,
              [key]: { ...prev, saving: true, saveReq: requestId, pendingSaveText: content, error: null },
            },
          }),
        }
      })
      if (get().design?.docs?.[key]?.saveReq !== requestId) return
      get().req({ r: 'FileSave', root, path, content, expectedFingerprint: file.fingerprint, requestId })
    },
    updateDesign: (root, path, doc: DesignDoc) => {
      const key = fileKey(root, path)
      set((s) => {
        const prev = s.design?.docs?.[key]
        if (!prev || prev.loading || prev.doc === doc) return s
        return {
          design: normalizeDesignSlice({
            ...s.design,
            docs: {
              ...s.design?.docs,
              [key]: { ...prev, doc, dirty: true },
            },
          }),
        }
      })
      // Serializing on every pointer move is the slow part of a drag, so the
      // back-to-saved check waits until edits pause.
      const pending = dirtyChecks.get(key)
      if (pending) clearTimeout(pending)
      dirtyChecks.set(key, setTimeout(() => {
        dirtyChecks.delete(key)
        set((s) => {
          const prev = s.design?.docs?.[key]
          if (!prev || prev.loading || !prev.dirty || prev.savedText == null) return s
          if (serializeDesign(prev.doc) !== prev.savedText) return s
          return {
            design: normalizeDesignSlice({
              ...s.design,
              docs: { ...s.design?.docs, [key]: { ...prev, dirty: false } },
            }),
          }
        })
      }, DIRTY_CHECK_MS))
    },
    createDesignFile: (root, path) => {
      if (get().design?.pendingCreate) return
      const createReq = mintRequestId()
      set((s) => ({
        design: normalizeDesignSlice({ ...s.design, pendingCreate: { root, path, createReq, readReq: null } }),
      }))
      get().req({ r: 'FileCreate', root, path, kind: 'file', requestId: createReq })
    },
  }
}

/** After FileCreate of a new design succeeds, open it and read the empty file. */
export function beginDesignSeed(set: StoreSet, get: StoreGet, root: string, path: string): void {
  const pending = get().design?.pendingCreate
  if (!pending || pending.root !== root || pending.path !== path) return
  const readReq = mintRequestId()
  const id = designTabId(root, path)
  const key = fileKey(root, path)
  set((s) => {
    const base = normalizeGroups(s.ui)
    const exists = base.tabs.some((t) => t.id === id)
    const tabs: Tab[] = exists
      ? base.tabs
      : [...base.tabs, { id, kind: 'design', root, path, title: baseName(path) }]
    return {
      ui: normalizeGroups({ ...base, tabs, activeTabId: id }),
      design: normalizeDesignSlice({
        ...s.design,
        panelTabId: id,
        pendingCreate: { root, path, createReq: pending.createReq, readReq },
        docs: {
          ...s.design?.docs,
          [key]: emptyDesignFile({ loading: true, readReq }),
        },
      }),
    }
  })
  get().req({ r: 'FileRead', root, path, requestId: readReq })
}
