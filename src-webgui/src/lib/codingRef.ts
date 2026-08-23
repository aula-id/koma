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

/**
 * Ask-in-chat payload: range token + fenced selection so the model sees the
 * actual buffer text (may differ from disk if dirty).
 */
export function codingAskInChatPayload(
  root: string,
  path: string,
  workdirs: string[],
  startLine: number,
  endLine: number,
  selectedText: string,
): string {
  const token = codingRangeToken(root, path, workdirs, startLine, endLine).trimEnd()
  const body = selectedText.replace(/\s+$/, '')
  if (!body) return `${token} `
  // Cap huge selections so the composer stays usable.
  const max = 12_000
  const clipped = body.length > max ? `${body.slice(0, max)}\n…` : body
  const fencePath = path.replace(/\\/g, '/')
  return `${token}\n\`\`\`${startLine}:${endLine}:${fencePath}\n${clipped}\n\`\`\`\n`
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
