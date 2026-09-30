// Open design buffers. Separate from coding.files so a .kdsgn never mounts Monaco.
import type { DesignDoc } from '../lib/design'
import { emptyDesign, parseDesign, serializeDesign } from '../lib/design'
import { fileKey, isPathOrDescendant, remapPath } from './coding'

export type DesignFileState = {
  doc: DesignDoc
  savedText: string | null
  fingerprint: string
  dirty: boolean
  saving: boolean
  loading: boolean
  error: string | null
  readReq: string | null
  saveReq: string | null
  pendingSaveText: string | null
}

export type DesignPendingCreate = {
  root: string
  path: string
  createReq: string
  readReq: string | null
}

export type DesignFileUiState = {
  selection: string[]
  focusId: string | null
  overrideTargetId: string | null
}

export type DesignSlice = {
  docs: Record<string, DesignFileState>
  pendingCreate: DesignPendingCreate | null
  /** Design tab bound to the sidebar (survives switching to Chat). */
  panelTabId: string | null
  fileUi: Record<string, DesignFileUiState>
}

export const emptyDesignFileUi = (): DesignFileUiState => ({
  selection: [],
  focusId: null,
  overrideTargetId: null,
})

export const initialDesign: DesignSlice = {
  docs: {},
  pendingCreate: null,
  panelTabId: null,
  fileUi: {},
}

/** Fill missing design-slice fields so UI never indexes undefined. */
export function normalizeDesignSlice(design: Partial<DesignSlice> | null | undefined): DesignSlice {
  return {
    docs: design?.docs ?? {},
    pendingCreate: design?.pendingCreate ?? null,
    panelTabId: design?.panelTabId ?? null,
    fileUi: design?.fileUi ?? {},
  }
}

export function dropDesignFileUi(
  fileUi: Record<string, DesignFileUiState> | null | undefined,
  root: string,
  path: string,
): Record<string, DesignFileUiState> {
  if (!fileUi) return {}
  const key = fileKey(root, path)
  if (!(key in fileUi)) return fileUi
  const next = { ...fileUi }
  delete next[key]
  return next
}

export function emptyDesignFile(partial?: Partial<DesignFileState>): DesignFileState {
  return {
    doc: emptyDesign(),
    savedText: null,
    fingerprint: '',
    dirty: false,
    saving: false,
    loading: false,
    error: null,
    readReq: null,
    saveReq: null,
    pendingSaveText: null,
    ...partial,
  }
}

type ReadEnv = {
  root: string
  path: string
  requestId: string
  content: string | null
  fingerprint: string
  binary: boolean
  tooLarge: boolean
  error: string | null
}

type SaveEnv = {
  root: string
  path: string
  requestId: string
  fingerprint: string
  error: string | null
}

export type DesignReadClaim = {
  design: DesignSlice
  save?: { root: string; path: string; content: string; fingerprint: string; requestId: string }
}

/** Claim a FileRead that this slice issued. Null means another buffer owns it. */
export function claimDesignRead(design: DesignSlice, env: ReadEnv): DesignReadClaim | null {
  const key = fileKey(env.root, env.path)
  const prev = design?.docs?.[key]
  if (!prev || prev.readReq !== env.requestId) return null
  const seeding = design.pendingCreate?.readReq === env.requestId
  if (env.error || env.binary || env.tooLarge || env.content == null) {
    return {
      design: normalizeDesignSlice({
        ...design,
        pendingCreate: seeding ? null : design.pendingCreate,
        docs: {
          ...design.docs,
          [key]: {
            ...prev,
            loading: false,
            readReq: null,
            doc: emptyDesign(),
            dirty: false,
            error: env.error ?? (env.binary ? 'This file is not text' : env.tooLarge ? 'This file is too large' : 'Could not read this design'),
          },
        },
      }),
    }
  }
  // A just-created file is empty. Fill it with an empty design, using the
  // fingerprint the host just reported so the following save is not a conflict.
  if (seeding && env.content === '') {
    const doc = emptyDesign()
    const content = serializeDesign(doc)
    const requestId = `dsgn-save-${env.requestId}`
    return {
      design: normalizeDesignSlice({
        ...design,
        pendingCreate: null,
        docs: {
          ...design.docs,
          [key]: emptyDesignFile({
            doc,
            fingerprint: env.fingerprint,
            dirty: true,
            saving: true,
            saveReq: requestId,
            pendingSaveText: content,
          }),
        },
      }),
      save: { root: env.root, path: env.path, content, fingerprint: env.fingerprint, requestId },
    }
  }
  const parsed = parseDesign(env.content)
  const text = serializeDesign(parsed.doc)
  return {
    design: normalizeDesignSlice({
      ...design,
      pendingCreate: seeding ? null : design.pendingCreate,
      docs: {
        ...design.docs,
        [key]: emptyDesignFile({
          doc: parsed.doc,
          savedText: text,
          fingerprint: env.fingerprint,
          error: parsed.error,
        }),
      },
    }),
  }
}

/** Claim a FileSave that this slice issued. Null means another buffer owns it. */
export function claimDesignSave(design: DesignSlice, env: SaveEnv): DesignSlice | null {
  const key = fileKey(env.root, env.path)
  const prev = design?.docs?.[key]
  if (!prev || prev.saveReq !== env.requestId) return null
  if (env.error) {
    return normalizeDesignSlice({
      ...design,
      docs: {
        ...design.docs,
        [key]: {
          ...prev,
          saving: false,
          saveReq: null,
          pendingSaveText: null,
          error: env.error,
        },
      },
    })
  }
  const latest = serializeDesign(prev.doc)
  const written = prev.pendingSaveText ?? latest
  return normalizeDesignSlice({
    ...design,
    docs: {
      ...design.docs,
      [key]: {
        ...prev,
        savedText: written,
        fingerprint: env.fingerprint || prev.fingerprint,
        dirty: latest !== written,
        saving: false,
        saveReq: null,
        pendingSaveText: null,
        error: null,
      },
    },
  })
}

export function remapDesignDocs(
  docs: Record<string, DesignFileState> | null | undefined,
  root: string,
  oldPath: string,
  newPath: string,
): Record<string, DesignFileState> {
  if (!docs) return {}
  let changed = false
  const next: Record<string, DesignFileState> = {}
  const prefix = `${root}:`
  for (const [key, doc] of Object.entries(docs)) {
    if (!key.startsWith(prefix)) {
      next[key] = doc
      continue
    }
    const mapped = remapPath(key.slice(prefix.length), oldPath, newPath)
    if (mapped == null) {
      next[key] = doc
      continue
    }
    next[fileKey(root, mapped)] = doc
    changed = true
  }
  return changed ? next : docs
}

export function dropDesignDocs(
  docs: Record<string, DesignFileState> | null | undefined,
  root: string,
  path: string,
): Record<string, DesignFileState> {
  if (!docs) return {}
  let changed = false
  const next: Record<string, DesignFileState> = {}
  const prefix = `${root}:`
  for (const [key, doc] of Object.entries(docs)) {
    if (key.startsWith(prefix) && isPathOrDescendant(key.slice(prefix.length), path)) {
      changed = true
      continue
    }
    next[key] = doc
  }
  return changed ? next : docs
}
