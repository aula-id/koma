// Open diagram buffers. Separate from coding.files so a .diag never mounts Monaco.
import type { DiagramDoc } from '../lib/diagram'
import { emptyDiagram, parseDiagram, serializeDiagram } from '../lib/diagram'
import { fileKey, isPathOrDescendant, remapPath } from './coding'

export type DiagramFileState = {
  doc: DiagramDoc
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

export type DiagramPendingCreate = {
  root: string
  path: string
  createReq: string
  readReq: string | null
}

export type DiagramSlice = {
  docs: Record<string, DiagramFileState>
  pendingCreate: DiagramPendingCreate | null
}

export const initialDiagram: DiagramSlice = {
  docs: {},
  pendingCreate: null,
}

export function emptyDiagramFile(partial?: Partial<DiagramFileState>): DiagramFileState {
  return {
    doc: emptyDiagram(),
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

export type DiagramReadClaim = {
  diagram: DiagramSlice
  save?: { root: string; path: string; content: string; fingerprint: string; requestId: string }
}

/** Claim a FileRead that this slice issued. Null means the coding buffer owns it. */
export function claimDiagramRead(diagram: DiagramSlice, env: ReadEnv): DiagramReadClaim | null {
  const key = fileKey(env.root, env.path)
  const prev = diagram.docs[key]
  if (!prev || prev.readReq !== env.requestId) return null
  const seeding = diagram.pendingCreate?.readReq === env.requestId
  if (env.error || env.binary || env.tooLarge || env.content == null) {
    return {
      diagram: {
        ...diagram,
        pendingCreate: seeding ? null : diagram.pendingCreate,
        docs: {
          ...diagram.docs,
          [key]: {
            ...prev,
            loading: false,
            readReq: null,
            doc: emptyDiagram(),
            dirty: false,
            error: env.error ?? (env.binary ? 'This file is not text' : env.tooLarge ? 'This file is too large' : 'Could not read this diagram'),
          },
        },
      },
    }
  }
  // A just-created file is empty. Fill it with an empty diagram, using the
  // fingerprint the host just reported so the following save is not a conflict.
  if (seeding && env.content === '') {
    const doc = emptyDiagram()
    const content = serializeDiagram(doc)
    const requestId = `diag-save-${env.requestId}`
    return {
      diagram: {
        pendingCreate: null,
        docs: {
          ...diagram.docs,
          [key]: emptyDiagramFile({
            doc,
            fingerprint: env.fingerprint,
            dirty: true,
            saving: true,
            saveReq: requestId,
            pendingSaveText: content,
          }),
        },
      },
      save: { root: env.root, path: env.path, content, fingerprint: env.fingerprint, requestId },
    }
  }
  const parsed = parseDiagram(env.content)
  const text = serializeDiagram(parsed.doc)
  return {
    diagram: {
      ...diagram,
      pendingCreate: seeding ? null : diagram.pendingCreate,
      docs: {
        ...diagram.docs,
        [key]: emptyDiagramFile({
          doc: parsed.doc,
          savedText: text,
          fingerprint: env.fingerprint,
          error: parsed.error,
        }),
      },
    },
  }
}

/** Claim a FileSave that this slice issued. Null means the coding buffer owns it. */
export function claimDiagramSave(diagram: DiagramSlice, env: SaveEnv): DiagramSlice | null {
  const key = fileKey(env.root, env.path)
  const prev = diagram.docs[key]
  if (!prev || prev.saveReq !== env.requestId) return null
  if (env.error) {
    return {
      ...diagram,
      docs: {
        ...diagram.docs,
        [key]: {
          ...prev,
          saving: false,
          saveReq: null,
          pendingSaveText: null,
          error: env.error,
        },
      },
    }
  }
  const latest = serializeDiagram(prev.doc)
  const written = prev.pendingSaveText ?? latest
  return {
    ...diagram,
    docs: {
      ...diagram.docs,
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
  }
}

export function remapDiagramDocs(
  docs: Record<string, DiagramFileState>,
  root: string,
  oldPath: string,
  newPath: string,
): Record<string, DiagramFileState> {
  let changed = false
  const next: Record<string, DiagramFileState> = {}
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

export function dropDiagramDocs(
  docs: Record<string, DiagramFileState>,
  root: string,
  path: string,
): Record<string, DiagramFileState> {
  let changed = false
  const next: Record<string, DiagramFileState> = {}
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
