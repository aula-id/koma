import { diagramTabId, parseDiagram, serializeDiagram, type DiagramDoc } from '../../lib/diagram'
import { syncDiagramNoteAssets } from '../../lib/diagramNoteAttach'
import { publishDiagramNotes } from '../../lib/diagramNotes'
import type { StoreGet, StoreSet } from '../api'
import { baseName, fileKey, mintRequestId } from '../coding'
import { emptyDiagramFile } from '../diagram'
import { normalizeGroups } from '../editorGroups'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function diagramActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openDiagramTab' | 'saveDiagram' | 'updateDiagram' | 'createDiagramFile'> {
  return {
    openDiagramTab: (root, path) => {
      const id = diagramTabId(root, path)
      const key = fileKey(root, path)
      const existing = get().diagram.docs[key]
      // A buffer that is loading, saving, dirty, or already read just gets focused.
      // A missing buffer, or a failed read with nothing stored, fetches again.
      const readReq =
        existing && (existing.loading || existing.saving || existing.dirty || existing.savedText != null)
          ? null
          : mintRequestId()
      set((s) => {
        const base = normalizeGroups(s.ui)
        const exists = base.tabs.some((t) => t.id === id)
        const tabs: Tab[] = exists
          ? base.tabs
          : [...base.tabs, { id, kind: 'diagram', root, path, title: baseName(path) }]
        const ui = normalizeGroups({ ...base, tabs, activeTabId: id })
        if (!readReq) return { ui }
        const prev = s.diagram.docs[key]
        return {
          ui,
          diagram: {
            ...s.diagram,
            docs: {
              ...s.diagram.docs,
              [key]: emptyDiagramFile({
                ...(prev ?? {}),
                loading: true,
                readReq,
                error: null,
              }),
            },
          },
        }
      })
      if (readReq) get().req({ r: 'FileRead', root, path, requestId: readReq })
    },
    saveDiagram: (root, path) => {
      const key = fileKey(root, path)
      const file = get().diagram.docs[key]
      if (!file || file.loading || file.saving) return
      const content = serializeDiagram(file.doc)
      const requestId = mintRequestId()
      set((s) => {
        const prev = s.diagram.docs[key]
        if (!prev || prev.loading || prev.saving) return s
        return {
          diagram: {
            ...s.diagram,
            docs: {
              ...s.diagram.docs,
              [key]: { ...prev, saving: true, saveReq: requestId, pendingSaveText: content, error: null },
            },
          },
        }
      })
      if (get().diagram.docs[key]?.saveReq !== requestId) return
      const previous = file.savedText ? parseDiagram(file.savedText).doc : null
      publishDiagramNotes(get().req, root, path, file.doc, { previous })
      void syncDiagramNoteAssets({ hostId: get().remoteState.hostId ?? 'local', root }, path, file.doc)
      get().req({ r: 'FileSave', root, path, content, expectedFingerprint: file.fingerprint, requestId })
    },
    updateDiagram: (root, path, doc: DiagramDoc) => {
      const key = fileKey(root, path)
      const text = serializeDiagram(doc)
      set((s) => {
        const prev = s.diagram.docs[key]
        if (!prev || prev.loading) return s
        const dirty = text !== (prev.savedText ?? '')
        if (serializeDiagram(prev.doc) === text && prev.dirty === dirty) return s
        return {
          diagram: {
            ...s.diagram,
            docs: {
              ...s.diagram.docs,
              [key]: { ...prev, doc, dirty },
            },
          },
        }
      })
    },
    createDiagramFile: (root, path) => {
      if (get().diagram.pendingCreate) return
      const createReq = mintRequestId()
      set((s) => ({
        diagram: { ...s.diagram, pendingCreate: { root, path, createReq, readReq: null } },
      }))
      get().req({ r: 'FileCreate', root, path, kind: 'file', requestId: createReq })
    },
  }
}

/** After FileCreate of a new diagram succeeds, open it and read the empty file. */
export function beginDiagramSeed(set: StoreSet, get: StoreGet, root: string, path: string): void {
  const pending = get().diagram.pendingCreate
  if (!pending || pending.root !== root || pending.path !== path) return
  const readReq = mintRequestId()
  const id = diagramTabId(root, path)
  const key = fileKey(root, path)
  set((s) => {
    const base = normalizeGroups(s.ui)
    const exists = base.tabs.some((t) => t.id === id)
    const tabs: Tab[] = exists
      ? base.tabs
      : [...base.tabs, { id, kind: 'diagram', root, path, title: baseName(path) }]
    return {
      ui: normalizeGroups({ ...base, tabs, activeTabId: id }),
      diagram: {
        pendingCreate: { root, path, createReq: pending.createReq, readReq },
        docs: {
          ...s.diagram.docs,
          [key]: emptyDiagramFile({ loading: true, readReq }),
        },
      },
    }
  })
  get().req({ r: 'FileRead', root, path, requestId: readReq })
}
