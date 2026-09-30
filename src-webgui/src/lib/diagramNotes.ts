// Per-shape notes under `.koma/<stem>/cards/<id>/` and `.koma/<stem>/lines/<id>/`.
// Source of truth remains each node/edge `detail` field in the `.diag` file.
import { mintRequestId } from '../store/coding'
import { isDiagramPath, type DiagramDoc, type DiagramNode } from './diagram'

const quietFileOps = new Set<string>()

export type DiagramShapeKind = 'node' | 'edge'

export function diagramStem(diagPath: string): string | null {
  if (!isDiagramPath(diagPath)) return null
  const stem = diagPath.slice('.koma/'.length).replace(/\.diag$/i, '')
  return stem || null
}

/** Diagram sidecar root: `.koma/<stem>/`. */
export function diagramFolder(diagPath: string): string | null {
  const stem = diagramStem(diagPath)
  return stem ? `.koma/${stem}` : null
}

/** @deprecated Legacy aggregate notes path; no longer written on save. */
export function diagramNotesPath(diagPath: string): string | null {
  const folder = diagramFolder(diagPath)
  return folder ? `${folder}/notes.md` : null
}

/** Asset + notes folder for one card or line. */
export function diagramShapeFolder(diagPath: string, kind: DiagramShapeKind, id: string): string | null {
  const base = diagramFolder(diagPath)
  if (!base || !id.trim()) return null
  const segment = kind === 'node' ? 'cards' : 'lines'
  return `${base}/${segment}/${id}`
}

export function diagramShapeNotesPath(diagPath: string, kind: DiagramShapeKind, id: string): string | null {
  const folder = diagramShapeFolder(diagPath, kind, id)
  return folder ? `${folder}/notes.md` : null
}

export function diagramHasDetails(doc: DiagramDoc): boolean {
  return doc.nodes.some((node) => !!node.detail?.trim()) || doc.edges.some((edge) => !!edge.detail?.trim())
}

function oneLine(text: string): string {
  return text.replace(/[`\r\n#]/g, ' ').replace(/\s+/g, ' ').trim()
}

export function shapeHeadingFor(text: string, id: string): string {
  return oneLine(text) || id
}

function nodeRef(doc: DiagramDoc, id: string): string {
  const node = doc.nodes.find((item) => item.id === id)
  if (!node) return `\`${id}\``
  const label = oneLine(node.text)
  return label ? `${label} (\`${id}\`)` : `\`${id}\``
}

function linkTitle(from: DiagramNode | undefined, to: DiagramNode | undefined, id: string): string {
  const a = from ? oneLine(from.text) || from.id : ''
  const b = to ? oneLine(to.text) || to.id : ''
  if (a && b) return `${a} to ${b}`
  return id
}

function shapeFileBody(title: string, meta: string[], body: string): string {
  return [`# ${title}`, '', ...meta, '', body.trim(), ''].join('\n')
}

/** Markdown for one shape's sidecar `notes.md`, or '' when no detail. */
export function renderShapeNote(doc: DiagramDoc, kind: DiagramShapeKind, id: string): string {
  if (kind === 'node') {
    const node = doc.nodes.find((item) => item.id === id)
    const body = node?.detail?.trim()
    if (!node || !body) return ''
    return shapeFileBody(shapeHeadingFor(node.text, node.id), [`id: ${node.id}`, `kind: ${node.kind}`], body)
  }
  const edge = doc.edges.find((item) => item.id === id)
  const body = edge?.detail?.trim()
  if (!edge || !body) return ''
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  const label = edge.text?.trim() || linkTitle(byId.get(edge.from), byId.get(edge.to), edge.id)
  return shapeFileBody(shapeHeadingFor(label, edge.id), [
    `id: ${edge.id}`,
    'kind: line',
    `from: ${nodeRef(doc, edge.from)}`,
    `to: ${nodeRef(doc, edge.to)}`,
  ], body)
}

/** Human-facing attachment filename for chat piles. */
export function shapeNoteAttachName(doc: DiagramDoc, kind: DiagramShapeKind, id: string): string {
  const slug = (text: string) =>
    oneLine(text)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 40) || id
  if (kind === 'node') {
    const node = doc.nodes.find((item) => item.id === id)
    const base = node ? slug(node.text) : id
    return `${base}-${id}-notes.md`
  }
  const edge = doc.edges.find((item) => item.id === id)
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  const label = edge?.text?.trim() || (edge ? linkTitle(byId.get(edge.from), byId.get(edge.to), edge.id) : id)
  const base = edge ? slug(label) : id
  return `line-${base}-${id}-notes.md`
}

export type DiagramShapeRef = { kind: DiagramShapeKind; id: string }

/** Shapes that have detail text in this doc. */
export function shapesWithDetail(doc: DiagramDoc): DiagramShapeRef[] {
  const out: DiagramShapeRef[] = []
  for (const node of doc.nodes) {
    if (node.detail?.trim()) out.push({ kind: 'node', id: node.id })
  }
  for (const edge of doc.edges) {
    if (edge.detail?.trim()) out.push({ kind: 'edge', id: edge.id })
  }
  return out
}

/** Union of shapes with detail in `doc` or `previous`. */
export function shapesNeedingNoteSync(doc: DiagramDoc, previous?: DiagramDoc | null): DiagramShapeRef[] {
  const keys = new Set<string>()
  const add = (ref: DiagramShapeRef) => keys.add(`${ref.kind}:${ref.id}`)
  for (const ref of shapesWithDetail(doc)) add(ref)
  if (previous) {
    for (const ref of shapesWithDetail(previous)) add(ref)
  }
  return [...keys].map((key) => {
    const colon = key.indexOf(':')
    return { kind: key.slice(0, colon) as DiagramShapeKind, id: key.slice(colon + 1) }
  })
}

/** Concatenated view of all shape notes (tests / search). */
export function renderDiagramNotes(doc: DiagramDoc, _diagPath?: string): string {
  const parts: string[] = []
  for (const { kind, id } of shapesWithDetail(doc)) {
    const section = renderShapeNote(doc, kind, id)
    if (section) parts.push(section)
  }
  return parts.length ? `${parts.join('\n')}\n` : ''
}

type WriteReq = (body: {
  r: 'FileWriteBytes'
  root: string
  path: string
  bytesB64: string
  overwrite?: boolean
  requestId: string
}) => void

type DeleteReq = (body: { r: 'FileDelete'; root: string; path: string; requestId: string }) => void

export function utf8ToBase64(text: string): string {
  const bytes = new TextEncoder().encode(text)
  return bytesToBase64(bytes)
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
  return btoa(binary)
}

/** Write per-shape `notes.md` (and clear removed shapes). */
export function publishDiagramShapeNotes(
  req: WriteReq & DeleteReq,
  root: string,
  diagPath: string,
  doc: DiagramDoc,
  options?: { previous?: DiagramDoc | null },
): void {
  if (!diagramFolder(diagPath)) return
  const previous = options?.previous ?? null
  for (const { kind, id } of shapesNeedingNoteSync(doc, previous)) {
    const path = diagramShapeNotesPath(diagPath, kind, id)
    if (!path) continue
    const body = renderShapeNote(doc, kind, id)
    if (body) {
      writeWorkspaceText(req, root, path, body)
    } else {
      const requestId = mintRequestId()
      expectMissingFileOp(requestId)
      req({ r: 'FileDelete', root, path, requestId })
    }
  }
}

/** @deprecated Use publishDiagramShapeNotes */
export function publishDiagramNotes(
  req: WriteReq & DeleteReq,
  root: string,
  diagPath: string,
  doc: DiagramDoc,
  options?: { previous?: DiagramDoc | null },
): void {
  publishDiagramShapeNotes(req, root, diagPath, doc, options)
}

export function writeWorkspaceText(req: WriteReq, root: string, path: string, text: string): void {
  req({
    r: 'FileWriteBytes',
    root,
    path,
    bytesB64: utf8ToBase64(text),
    overwrite: true,
    requestId: mintRequestId(),
  })
}

export function writeWorkspaceBytes(req: WriteReq, root: string, path: string, bytes: Uint8Array): void {
  req({
    r: 'FileWriteBytes',
    root,
    path,
    bytesB64: bytesToBase64(bytes),
    overwrite: true,
    requestId: mintRequestId(),
  })
}

export function expectMissingFileOp(requestId: string): void {
  quietFileOps.add(requestId)
}

export function consumeQuietMiss(requestId: string, error: string | null | undefined): boolean {
  const ours = quietFileOps.delete(requestId)
  return ours && !!error && /does not exist/i.test(error)
}

/** Image names referenced in one shape's detail markdown. */
export function shapeDetailImageNames(detail: string | undefined): string[] {
  const names = new Set<string>()
  if (!detail) return []
  const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|svg)$/i
  const basename = (path: string) => path.replace(/\\/g, '/').trim().split('/').pop() ?? path
  for (const m of detail.matchAll(/!\[[^\]]*]\(([^)]+)\)/g)) {
    const base = basename(m[1] ?? '')
    if (IMAGE_EXT.test(base)) names.add(base)
  }
  for (const m of detail.matchAll(/\b([A-Za-z0-9][A-Za-z0-9._-]*\.(?:png|jpe?g|gif|webp|bmp|svg))\b/gi)) {
    names.add(m[1] ?? '')
  }
  return [...names]
}

export function shapeDetailText(doc: DiagramDoc, kind: DiagramShapeKind, id: string): string | undefined {
  if (kind === 'node') return doc.nodes.find((n) => n.id === id)?.detail
  return doc.edges.find((e) => e.id === id)?.detail
}
