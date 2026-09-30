// When a diagram is sent to chat, stage per-shape notes + images as composer attachments.

import type { DiagramDoc } from './diagram'
import {
  bytesToBase64,
  cachedDiagramNoteImage,
  cachedDiagramNoteImageByName,
  diagramFolder,
  diagramShapeFolder,
  renderShapeNote,
  shapeDetailImageNames,
  shapeDetailText,
  shapesWithDetail,
  writeWorkspaceBytes,
  type DiagramShapeKind,
  type DiagramShapeRef,
} from './diagramNotes'
import {
  normalizeDiagramNoteMarkdown,
  noteImageFile,
  referencedNoteImageNames,
  repairComposerImageMarkers,
} from './markdownNote'
import { codingRequest, type WorkspaceRef } from './coding-service'
import { requestFileBytes } from './filePreview'
import { mintRequestId, type FileTreeEntry } from '../store/coding'
import { useKoma } from '../store/koma'

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|svg)$/i

function basename(path: string): string {
  const norm = path.replace(/\\/g, '/').trim()
  return norm.split('/').pop() ?? norm
}

/** Workspace image paths in a folder that are no longer referenced. */
export function orphanDiagramNoteImages(entries: FileTreeEntry[], referenced: Set<string>): string[] {
  const orphans: string[] = []
  for (const entry of entries) {
    if (entry.isDir) continue
    const name = basename(entry.path)
    if (name === 'notes.md') continue
    if (!IMAGE_EXT.test(name)) continue
    if (!referenced.has(name)) orphans.push(entry.path)
  }
  return orphans
}

function mimeFor(name: string): string | undefined {
  if (/\.png$/i.test(name)) return 'image/png'
  if (/\.jpe?g$/i.test(name)) return 'image/jpeg'
  if (/\.gif$/i.test(name)) return 'image/gif'
  if (/\.webp$/i.test(name)) return 'image/webp'
  if (/\.bmp$/i.test(name)) return 'image/bmp'
  if (/\.svg$/i.test(name)) return 'image/svg+xml'
  if (/\.md$/i.test(name)) return 'text/markdown'
  return undefined
}

type FileReq = (body: {
  r: 'AttachFile'
  name: string
  bytesB64: string
  mime?: string
}) => void

type DeleteReq = (body: { r: 'FileDelete'; root: string; path: string; requestId: string }) => void

type WriteReq = (body: {
  r: 'FileWriteBytes'
  root: string
  path: string
  bytesB64: string
  overwrite?: boolean
  requestId: string
}) => void

async function listFolder(workspace: WorkspaceRef, path: string): Promise<FileTreeEntry[]> {
  const requestId = mintRequestId()
  try {
    const result = await codingRequest<{ entries?: FileTreeEntry[] }>(workspace, {
      op: 'file',
      body: { r: 'FileTree', root: workspace.root, path, requestId },
    })
    return result.entries ?? []
  } catch {
    return []
  }
}

async function readBytes(
  req: Parameters<typeof requestFileBytes>[0],
  root: string,
  path: string,
): Promise<Uint8Array | null> {
  const cached = cachedDiagramNoteImage(root, path)
  if (cached?.length) return cached
  try {
    return await requestFileBytes(req, root, path)
  } catch {
    return null
  }
}

async function readShapeImage(
  req: Parameters<typeof requestFileBytes>[0],
  root: string,
  folder: string | null,
  legacy: string | null,
  name: string,
): Promise<Uint8Array | null> {
  const named = cachedDiagramNoteImageByName(root, name)
  if (named?.length) return named
  const paths = [folder ? `${folder}/${name}` : null, legacy ? `${legacy}/${name}` : null].filter(
    (path): path is string => !!path,
  )
  for (const path of paths) {
    const hit = await readBytes(req, root, path)
    if (hit?.length) return hit
  }
  for (const path of paths) {
    for (let attempt = 0; attempt < 3; attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 60 * (attempt + 1)))
      const hit = await readBytes(req, root, path)
      if (hit?.length) return hit
    }
  }
  return null
}

function imageNamesInShape(detail: string | undefined, note: string): string[] {
  const names = new Set<string>()
  for (const source of [detail, note]) {
    for (const name of shapeDetailImageNames(source)) names.add(name)
  }
  return [...names]
}

/** Copy legacy flat `.koma/<stem>/img.png` into a shape folder when referenced only there. */
export async function migrateLegacyShapeImages(
  workspace: WorkspaceRef,
  diagPath: string,
  doc: DiagramDoc,
  req: WriteReq,
): Promise<void> {
  const legacy = diagramFolder(diagPath)
  if (!legacy) return
  const legacyEntries = await listFolder(workspace, legacy)
  const legacyImages = new Set(
    legacyEntries.filter((e) => !e.isDir && noteImageFile(basename(e.path))).map((e) => basename(e.path)),
  )
  if (!legacyImages.size) return
  const download = useKoma.getState().req as Parameters<typeof requestFileBytes>[0]
  for (const { kind, id } of shapesWithDetail(doc)) {
    const folder = diagramShapeFolder(diagPath, kind, id)
    if (!folder) continue
    const detail = shapeDetailText(doc, kind, id) ?? ''
    for (const name of shapeDetailImageNames(detail)) {
      if (!legacyImages.has(name)) continue
      const dest = `${folder}/${name}`
      const existing = await readBytes(download, workspace.root, dest)
      if (existing?.length) continue
      const bytes = await readBytes(download, workspace.root, `${legacy}/${name}`)
      if (!bytes?.length) continue
      writeWorkspaceBytes(req, workspace.root, dest, bytes)
    }
  }
}

/** Stage each shape's notes + images as session attachment piles. */
export async function attachDiagramShapesToComposer(root: string, diagPath: string, doc: DiagramDoc): Promise<void> {
  const st = useKoma.getState()
  const req = st.req
  for (const { kind, id } of shapesWithDetail(doc)) {
    const folder = diagramShapeFolder(diagPath, kind, id)
    const body = renderShapeNote(doc, kind, id)
    if (body.trim()) {
      st.stageComposerAttachmentInsert('pasted_text')
      req({ r: 'AttachPaste', text: body })
    }
    const detail = shapeDetailText(doc, kind, id)
    let names = imageNamesInShape(detail, body)
    const legacy = diagramFolder(diagPath)
    if (!names.length && folder) {
      const listed = await listFolder({ hostId: useKoma.getState().remoteState.hostId ?? 'local', root }, folder)
      const files = listed.filter((entry) => !entry.isDir && noteImageFile(basename(entry.path))).map((entry) => basename(entry.path))
      if (/\[Image #\d+\]/.test(detail ?? '') || /\[Image #\d+\]/.test(body)) {
        names = files
      } else if (files.length && /!\[[^\]]*]\([^)]+\)/.test(`${detail ?? ''}\n${body}`)) {
        names = files
      }
    }
    for (const name of names) {
      const bytes = await readShapeImage(req, root, folder, legacy, name)
      if (!bytes?.length) continue
      st.stageComposerAttachmentInsert('image')
      req({
        r: 'AttachFile',
        name,
        bytesB64: bytesToBase64(bytes),
        mime: mimeFor(name),
      })
    }
  }
}

/** Load/repair diagram detail markdown (composer markers → workspace images). */
export async function prepareDiagramNoteMarkdown(
  workspace: WorkspaceRef,
  assetDir: string,
  markdown: string,
  legacyAssetDir?: string | null,
): Promise<string> {
  let md = markdown
  const dirs = [assetDir, legacyAssetDir].filter((d): d is string => !!d)
  if (/\[Image #\d+\]/.test(md)) {
    const files = new Set<string>()
    for (const dir of dirs) {
      const listed = await listFolder(workspace, dir)
      for (const entry of listed) {
        if (!entry.isDir && noteImageFile(basename(entry.path))) files.add(basename(entry.path))
      }
    }
    const spare = [...files].filter((name) => !referencedNoteImageNames(md).has(name))
    md = repairComposerImageMarkers(md, spare)
  }
  return normalizeDiagramNoteMarkdown(md)
}

async function syncOneShapeFolder(
  workspace: WorkspaceRef,
  diagPath: string,
  kind: DiagramShapeKind,
  id: string,
  doc: DiagramDoc,
  req: DeleteReq,
): Promise<void> {
  const folder = diagramShapeFolder(diagPath, kind, id)
  if (!folder) return
  const detail = shapeDetailText(doc, kind, id) ?? ''
  const referenced = new Set(shapeDetailImageNames(detail))
  const entries = await listFolder(workspace, folder)
  const orphans = orphanDiagramNoteImages(entries, referenced)
  for (const path of orphans) {
    req({ r: 'FileDelete', root: workspace.root, path, requestId: mintRequestId() })
  }
}

/** Per-shape orphan GC + legacy flat-folder cleanup. */
export async function syncDiagramShapeNotes(
  workspace: WorkspaceRef,
  diagPath: string,
  doc: DiagramDoc,
  options?: { previous?: DiagramDoc | null },
): Promise<void> {
  const req = useKoma.getState().req as DeleteReq & WriteReq
  await migrateLegacyShapeImages(workspace, diagPath, doc, req)
  const previous = options?.previous ?? null
  const refs: DiagramShapeRef[] = []
  const seen = new Set<string>()
  const add = (ref: DiagramShapeRef) => {
    const key = `${ref.kind}:${ref.id}`
    if (seen.has(key)) return
    seen.add(key)
    refs.push(ref)
  }
  for (const ref of shapesWithDetail(doc)) add(ref)
  if (previous) {
    for (const node of previous.nodes) {
      if (node.detail?.trim()) add({ kind: 'node', id: node.id })
    }
    for (const edge of previous.edges) {
      if (edge.detail?.trim()) add({ kind: 'edge', id: edge.id })
    }
  }
  for (const ref of refs) {
    await syncOneShapeFolder(workspace, diagPath, ref.kind, ref.id, doc, req)
  }
  const legacy = diagramFolder(diagPath)
  if (!legacy) return
  const allReferenced = new Set<string>()
  for (const node of doc.nodes) shapeDetailImageNames(node.detail).forEach((n) => allReferenced.add(n))
  for (const edge of doc.edges) shapeDetailImageNames(edge.detail).forEach((n) => allReferenced.add(n))
  const legacyEntries = await listFolder(workspace, legacy)
  const legacyOrphans = legacyEntries
    .filter((e) => !e.isDir && noteImageFile(basename(e.path)))
    .map((e) => e.path)
    .filter((path) => {
      const name = basename(path)
      if (name === 'notes.md') return true
      return !allReferenced.has(name)
    })
  for (const path of legacyOrphans) {
    req({ r: 'FileDelete', root: workspace.root, path, requestId: mintRequestId() })
  }
}

/** @deprecated Use syncDiagramShapeNotes */
export async function syncDiagramNoteAssets(workspace: WorkspaceRef, diagPath: string, doc: DiagramDoc): Promise<void> {
  await syncDiagramShapeNotes(workspace, diagPath, doc)
}
