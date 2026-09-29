// Readable notes for a diagram. The editor keeps each description on the shape.
// Saving the diagram also writes `.koma/<stem>/notes.md`, and pictures live in
// that same folder so a person or the agent can open them.
import { mintRequestId } from '../store/coding'
import { isDiagramPath, type DiagramDoc, type DiagramNode } from './diagram'

const quietFileOps = new Set<string>()

export function diagramStem(diagPath: string): string | null {
  if (!isDiagramPath(diagPath)) return null
  const stem = diagPath.slice('.koma/'.length).replace(/\.diag$/i, '')
  return stem || null
}

/** Folder next to the diagram: `.koma/<stem>/`. */
export function diagramFolder(diagPath: string): string | null {
  const stem = diagramStem(diagPath)
  return stem ? `.koma/${stem}` : null
}

export function diagramNotesPath(diagPath: string): string | null {
  const folder = diagramFolder(diagPath)
  return folder ? `${folder}/notes.md` : null
}

export function diagramHasDetails(doc: DiagramDoc): boolean {
  return doc.nodes.some((node) => !!node.detail?.trim()) || doc.edges.some((edge) => !!edge.detail?.trim())
}

function oneLine(text: string): string {
  return text.replace(/[`\r\n#]/g, ' ').replace(/\s+/g, ' ').trim()
}

function headingFor(text: string, id: string): string {
  return oneLine(text) || id
}

function nodeRef(doc: DiagramDoc, id: string): string {
  const node = doc.nodes.find((item) => item.id === id)
  if (!node) return `\`${id}\``
  const label = oneLine(node.text)
  return label ? `${label} (\`${id}\`)` : `\`${id}\``
}

function header(diagPath: string): string {
  const stem = diagramStem(diagPath) || 'diagram'
  const folder = diagramFolder(diagPath) || `.koma/${stem}`
  return [
    `# ${stem}`,
    '',
    `Notes for \`${diagPath}\`. Each section is one card or line. A picture \`img.png\` in a section is the workspace file \`${folder}/img.png\`.`,
  ].join('\n')
}

function section(title: string, lines: string[], body: string): string {
  return [`## ${title}`, '', ...lines, '', body.trim()].join('\n')
}

/** Full notes file, or an empty string when nothing is described. */
export function renderDiagramNotes(doc: DiagramDoc, diagPath: string): string {
  const sections: string[] = []
  for (const node of doc.nodes) {
    const body = node.detail?.trim()
    if (!body) continue
    sections.push(section(headingFor(node.text, node.id), [`id: ${node.id}`, `kind: ${node.kind}`], body))
  }
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  for (const edge of doc.edges) {
    const body = edge.detail?.trim()
    if (!body) continue
    const label = edge.text?.trim() || linkTitle(byId.get(edge.from), byId.get(edge.to), edge.id)
    sections.push(
      section(headingFor(label, edge.id), [`id: ${edge.id}`, 'kind: line', `from: ${nodeRef(doc, edge.from)}`, `to: ${nodeRef(doc, edge.to)}`], body),
    )
  }
  if (!sections.length) return ''
  return `${header(diagPath)}\n\n${sections.join('\n\n')}\n`
}

function linkTitle(from: DiagramNode | undefined, to: DiagramNode | undefined, id: string): string {
  const a = from ? oneLine(from.text) || from.id : ''
  const b = to ? oneLine(to.text) || to.id : ''
  if (a && b) return `${a} to ${b}`
  return id
}

export function renderDiagramNotesHeader(diagPath: string): string {
  return `${header(diagPath)}\n`
}

type WriteReq = (body: {
  r: 'FileWriteBytes'
  root: string
  path: string
  bytesB64: string
  overwrite?: boolean
  requestId: string
}) => void

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

/** Write the notes file when this diagram has descriptions, or when it used to. */
export function publishDiagramNotes(
  req: WriteReq,
  root: string,
  diagPath: string,
  doc: DiagramDoc,
  options?: { previous?: DiagramDoc | null },
): void {
  const path = diagramNotesPath(diagPath)
  if (!path) return
  const body = renderDiagramNotes(doc, diagPath)
  const previous = options?.previous ? renderDiagramNotes(options.previous, diagPath) : ''
  if (!body && !previous) return
  writeWorkspaceText(req, root, path, body || renderDiagramNotesHeader(diagPath))
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

/** A follow-up rename or delete may miss a notes folder that was never created. */
export function expectMissingFileOp(requestId: string): void {
  quietFileOps.add(requestId)
}

/** True when this reply is a notes-folder op whose path was not on disk. */
export function consumeQuietMiss(requestId: string, error: string | null | undefined): boolean {
  const ours = quietFileOps.delete(requestId)
  return ours && !!error && /does not exist/i.test(error)
}
