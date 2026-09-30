// When a diagram is sent to chat, stage notes + detail images as composer attachments.

import type { DiagramDoc } from './diagram'
import { bytesToBase64, diagramFolder, diagramStem, renderDiagramNotes, utf8ToBase64 } from './diagramNotes'
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

function collectImageNamesFromText(text: string, names: Set<string>) {
  for (const m of text.matchAll(/!\[[^\]]*]\(([^)]+)\)/g)) {
    const base = basename(m[1] ?? '')
    if (IMAGE_EXT.test(base)) names.add(base)
  }
  for (const m of text.matchAll(/\b([A-Za-z0-9][A-Za-z0-9._-]*\.(?:png|jpe?g|gif|webp|bmp|svg))\b/gi)) {
    names.add(m[1] ?? '')
  }
}

/** Image filenames referenced in diagram detail / generated notes markdown. */
export function diagramDetailImageNames(doc: DiagramDoc, diagPath: string): string[] {
  const names = new Set<string>()
  for (const node of doc.nodes) {
    if (node.detail) collectImageNamesFromText(node.detail, names)
  }
  for (const edge of doc.edges) {
    if (edge.detail) collectImageNamesFromText(edge.detail, names)
  }
  const notesBody = renderDiagramNotes(doc, diagPath)
  if (notesBody) collectImageNamesFromText(notesBody, names)
  return [...names]
}

/** Workspace image paths under `.koma/<stem>/` that are no longer referenced in notes. */
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

/** Stage generated notes as a session file attachment (composer pile). */
export async function attachDiagramNotesFile(
  root: string,
  diagPath: string,
  doc: DiagramDoc,
  options?: { preferWorkspaceFile?: boolean },
): Promise<void> {
  const body = renderDiagramNotes(doc, diagPath)
  if (!body.trim()) return
  const stem = diagramStem(diagPath) ?? 'diagram'
  const attachName = `${stem}-notes.md`
  const req = useKoma.getState().req as FileReq & Parameters<typeof requestFileBytes>[0]
  const folder = diagramFolder(diagPath)
  if (options?.preferWorkspaceFile && folder) {
    try {
      const bytes = await requestFileBytes(req, root, `${folder}/notes.md`)
      if (bytes.length) {
        req({ r: 'AttachFile', name: attachName, bytesB64: bytesToBase64(bytes), mime: 'text/markdown' })
        return
      }
    } catch {
      /* fall back to rendered bytes */
    }
  }
  req({ r: 'AttachFile', name: attachName, bytesB64: utf8ToBase64(body), mime: 'text/markdown' })
}

/** Stage detail images as session attachments (`[Image #N]` piles in the composer). */
export async function attachDiagramDetailImages(root: string, diagPath: string, doc: DiagramDoc): Promise<void> {
  const folder = diagramFolder(diagPath)
  if (!folder) return
  const names = diagramDetailImageNames(doc, diagPath)
  if (!names.length) return
  const req = useKoma.getState().req as FileReq & Parameters<typeof requestFileBytes>[0]
  for (const name of names) {
    const path = `${folder}/${name}`
    try {
      const bytes = await requestFileBytes(req, root, path)
      req({
        r: 'AttachFile',
        name,
        bytesB64: bytesToBase64(bytes),
        mime: mimeFor(name),
      })
    } catch {
      /* missing or unreadable — skip */
    }
  }
}

async function listDiagramAssetFiles(workspace: WorkspaceRef, assetDir: string): Promise<string[]> {
  const requestId = mintRequestId()
  try {
    const result = await codingRequest<{ entries?: FileTreeEntry[] }>(workspace, {
      op: 'file',
      body: { r: 'FileTree', root: workspace.root, path: assetDir, requestId },
    })
    return (result.entries ?? [])
      .filter((entry) => !entry.isDir && noteImageFile(basename(entry.path)))
      .map((entry) => basename(entry.path))
  } catch {
    return []
  }
}

/** Load/repair diagram detail markdown (composer markers → workspace images). */
export async function prepareDiagramNoteMarkdown(
  workspace: WorkspaceRef,
  assetDir: string,
  markdown: string,
): Promise<string> {
  let md = markdown
  if (/\[Image #\d+\]/.test(md) && assetDir) {
    const files = await listDiagramAssetFiles(workspace, assetDir)
    const spare = files.filter((name) => !referencedNoteImageNames(md).has(name))
    md = repairComposerImageMarkers(md, spare)
  }
  return normalizeDiagramNoteMarkdown(md)
}

/** Delete images in `.koma/<stem>/` that are no longer referenced in diagram notes. */
export async function syncDiagramNoteAssets(workspace: WorkspaceRef, diagPath: string, doc: DiagramDoc): Promise<void> {
  const folder = diagramFolder(diagPath)
  if (!folder) return
  const referenced = new Set(diagramDetailImageNames(doc, diagPath))
  const requestId = mintRequestId()
  let entries: FileTreeEntry[] = []
  try {
    const result = await codingRequest<{ entries?: FileTreeEntry[] }>(workspace, {
      op: 'file',
      body: { r: 'FileTree', root: workspace.root, path: folder, requestId },
    })
    entries = result.entries ?? []
  } catch {
    return
  }
  const orphans = orphanDiagramNoteImages(entries, referenced)
  if (!orphans.length) return
  const req = useKoma.getState().req as DeleteReq
  for (const path of orphans) {
    req({ r: 'FileDelete', root: workspace.root, path, requestId: mintRequestId() })
  }
}
