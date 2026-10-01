/** Shared helpers: coding paths → composer `@` tokens (TUI/omnisearch parity). */

/** Multi-root: `@[N]rel/path `; single-root: `@rel/path `. Trailing space for chip insert. */
export function codingRefToken(
  root: string,
  path: string,
  workdirs: string[],
  opts?: { isDir?: boolean },
): string {
  const rel = (path || '').replace(/\\/g, '/').replace(/^\/+/, '')
  const multi = workdirs.length > 1
  const idx = workdirs.indexOf(root)
  let label = multi && idx >= 0 ? `[${idx}]${rel}` : rel
  if (opts?.isDir && label && !label.endsWith('/')) label += '/'
  if (!label) {
    // Workspace root itself.
    label = multi && idx >= 0 ? `[${idx}]` : '.'
  }
  return `@${label} `
}

/** `@path:12` or `@path:12-40` (1-based, inclusive). */
export function codingRangeToken(
  root: string,
  path: string,
  workdirs: string[],
  startLine: number,
  endLine: number,
): string {
  const base = codingRefToken(root, path, workdirs).trimEnd()
  const a = Math.max(1, startLine)
  const b = Math.max(a, endLine)
  return b === a ? `${base}:${a} ` : `${base}:${a}-${b} `
}

/** Pile label: `file.rs:22` or `file.rs:62:67`. */
export function codingAskInChatLabel(path: string, startLine: number, endLine: number): string {
  const name = path.replace(/\\/g, '/').split('/').filter(Boolean).pop() || path || 'file'
  const a = Math.max(1, startLine)
  const b = Math.max(a, endLine)
  return b === a ? `${name}:${a}` : `${name}:${a}:${b}`
}

/** Selection staged as a paste pile. The composer hides the body until send. */
export function codingAskInChatPaste(
  path: string,
  startLine: number,
  endLine: number,
  selectedText: string,
): { text: string; label: string; path: string } | null {
  const text = selectedText.replace(/\s+$/, '')
  if (!text) return null
  const a = Math.max(1, startLine)
  const b = Math.max(a, endLine)
  const rel = path.replace(/\\/g, '/').replace(/^\/+/, '')
  const fencePath = b === a ? `${rel}:${a}` : `${rel}:${a}:${b}`
  return { text, label: codingAskInChatLabel(path, a, b), path: fencePath }
}

/** Custom MIME for tree → composer DnD (not external file upload). */
export const CODING_PATH_DND = 'application/x-koma-coding-path'

export type CodingPathDragPayload = {
  root: string
  path: string
  isDir: boolean
}

export function setCodingPathDragData(
  dt: DataTransfer,
  payload: CodingPathDragPayload,
): void {
  dt.setData(CODING_PATH_DND, JSON.stringify(payload))
  // Fallback plain text for debuggers / external drops.
  dt.setData('text/plain', payload.path)
  dt.effectAllowed = 'copy'
}

export function readCodingPathDragData(dt: DataTransfer): CodingPathDragPayload | null {
  const raw = dt.getData(CODING_PATH_DND)
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as CodingPathDragPayload
    if (!v || typeof v.root !== 'string' || typeof v.path !== 'string') return null
    return { root: v.root, path: v.path, isDir: !!v.isDir }
  } catch {
    return null
  }
}

export function hasCodingPathDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt?.types) return false
  for (let i = 0; i < dt.types.length; i++) {
    if (dt.types[i] === CODING_PATH_DND) return true
  }
  return false
}
