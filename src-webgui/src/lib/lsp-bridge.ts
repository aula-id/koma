// Monaco-free LSP types + pending-request map + URI helpers.
// The boot store and Problems drawer import THIS file so the main bundle
// never pulls monaco-editor. monaco-lsp.ts (lazy editor path) re-exports these.

export type LspDiagnostic = {
  uri: string
  line: number
  character: number
  endLine: number
  endCharacter: number
  severity: number
  message: string
  source?: string
  code?: string
}

export type LspTextEdit = {
  range: {
    startLine: number
    startCharacter: number
    endLine: number
    endCharacter: number
  }
  newText: string
}

export type LspCompletionItem = {
  label: string
  kind?: number
  detail?: string
  /** Secondary label (module path) from labelDetails.description. */
  labelDescription?: string
  insertText?: string
  /** 1=PlainText 2=Snippet */
  insertTextFormat?: number
  documentation?: string
  sortText?: string
  filterText?: string
  textEdit?: LspTextEdit
  /** Auto-import and other side edits applied on accept. */
  additionalTextEdits?: LspTextEdit[]
  /** Opaque server token for completionItem/resolve. */
  data?: unknown
  /** Characters that accept the item when typed. */
  commitCharacters?: string[]
  /** Prefer this item in the suggest widget. */
  preselect?: boolean
}

/** Result of textDocument/completion — items + incomplete re-query flag. */
export type LspCompletionList = {
  items: LspCompletionItem[]
  /** When true Monaco re-queries on the next keystroke (path completion). */
  isIncomplete: boolean
}

export type LspHover = {
  contents: string
  range?: {
    startLine: number
    startCharacter: number
    endLine: number
    endCharacter: number
  }
}

export type LspLocation = {
  uri: string
  range: {
    startLine: number
    startCharacter: number
    endLine: number
    endCharacter: number
  }
}

export type LspDocumentSymbol = {
  name: string
  kind: number
  range: {
    startLine: number
    startCharacter: number
    endLine: number
    endCharacter: number
  }
  selectionRange: {
    startLine: number
    startCharacter: number
    endLine: number
    endCharacter: number
  }
}

type Pending<T> = {
  resolve: (v: T) => void
  reject: (e: Error) => void
  timer: ReturnType<typeof setTimeout>
}

const PENDING_MS = 12_000

const pendingCompletion = new Map<string, Pending<LspCompletionList>>()
const pendingCompletionResolve = new Map<string, Pending<LspCompletionItem>>()
const pendingHover = new Map<string, Pending<LspHover | null>>()
const pendingDefinition = new Map<string, Pending<LspLocation[]>>()
const pendingReferences = new Map<string, Pending<LspLocation[]>>()
const pendingDocumentSymbol = new Map<string, Pending<LspDocumentSymbol[]>>()
const pendingFileText = new Map<string, Pending<string>>()

let reqCounter = 0
export function mintId(prefix: string): string {
  reqCounter += 1
  return `${prefix}-${Date.now().toString(36)}-${reqCounter}`
}

export function track<T>(map: Map<string, Pending<T>>, id: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      map.delete(id)
      reject(new Error('LSP request timed out'))
    }, PENDING_MS)
    map.set(id, { resolve, reject, timer })
  })
}

function settle<T>(
  map: Map<string, Pending<T>>,
  id: string,
  value: T,
  error: string | null | undefined,
): void {
  const p = map.get(id)
  if (!p) return
  map.delete(id)
  clearTimeout(p.timer)
  if (error) p.reject(new Error(error))
  else p.resolve(value)
}

export function trackCompletion(id: string): Promise<LspCompletionList> {
  return track(pendingCompletion, id)
}
export function trackCompletionResolve(id: string): Promise<LspCompletionItem> {
  return track(pendingCompletionResolve, id)
}
export function trackHover(id: string): Promise<LspHover | null> {
  return track(pendingHover, id)
}
export function trackDefinition(id: string): Promise<LspLocation[]> {
  return track(pendingDefinition, id)
}
export function trackReferences(id: string): Promise<LspLocation[]> {
  return track(pendingReferences, id)
}
export function trackDocumentSymbol(id: string): Promise<LspDocumentSymbol[]> {
  return track(pendingDocumentSymbol, id)
}
export function trackFileText(id: string): Promise<string> {
  return track(pendingFileText, id)
}

export function resolveLspCompletion(
  requestId: string,
  items: LspCompletionItem[],
  error: string | null,
  isIncomplete?: boolean | null,
): void {
  settle(
    pendingCompletion,
    requestId,
    { items: items ?? [], isIncomplete: !!isIncomplete },
    error,
  )
}

export function resolveLspCompletionResolve(
  requestId: string,
  item: LspCompletionItem | null,
  error: string | null,
): void {
  if (error) {
    settle(pendingCompletionResolve, requestId, {} as LspCompletionItem, error)
    return
  }
  settle(pendingCompletionResolve, requestId, item ?? ({} as LspCompletionItem), null)
}

export function resolveLspHover(
  requestId: string,
  hover: LspHover | null,
  error: string | null,
): void {
  settle(pendingHover, requestId, hover, error)
}

export function resolveLspDefinition(
  requestId: string,
  locations: LspLocation[],
  error: string | null,
): void {
  settle(pendingDefinition, requestId, locations ?? [], error)
}

export function resolveLspReferences(
  requestId: string,
  locations: LspLocation[],
  error: string | null,
): void {
  settle(pendingReferences, requestId, locations ?? [], error)
}

export function resolveLspDocumentSymbol(
  requestId: string,
  symbols: LspDocumentSymbol[],
  error: string | null,
): void {
  settle(pendingDocumentSymbol, requestId, symbols ?? [], error)
}

export function resolveLspFileText(
  requestId: string,
  content: string | null,
  error: string | null,
  binary?: boolean,
  tooLarge?: boolean,
): void {
  if (!pendingFileText.has(requestId)) return
  if (error) {
    settle(pendingFileText, requestId, '', error)
    return
  }
  if (binary || tooLarge) {
    settle(pendingFileText, requestId, '', binary ? 'binary file' : 'file too large')
    return
  }
  settle(pendingFileText, requestId, content ?? '', null)
}

/** file:// URI matching the host's path_to_uri. */
export function pathToUri(root: string, path: string): string {
  const abs = path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path)
    ? path
    : `${root.replace(/\/$/, '')}/${path.replace(/^\//, '')}`
  const norm = abs.replace(/\\/g, '/')
  if (norm.startsWith('/')) return `file://${norm}`
  return `file:///${norm}`
}

/** Best-effort parse of file:// URI → absolute filesystem path. */
export function uriToPath(uri: string): string | null {
  if (!uri.startsWith('file://')) return null
  let rest = uri.slice('file://'.length)
  // Windows file:///C:/...
  if (/^\/[A-Za-z]:\//.test(rest)) rest = rest.slice(1)
  try {
    return decodeURIComponent(rest)
  } catch {
    return rest
  }
}

/** Split absolute path into workspace root + relative path using open roots. */
export function splitRootPath(
  absPath: string,
  roots: string[],
): { root: string; path: string } | null {
  const norm = absPath.replace(/\\/g, '/')
  const sorted = [...roots].sort((a, b) => b.length - a.length)
  for (const root of sorted) {
    const r = root.replace(/\\/g, '/').replace(/\/$/, '')
    if (norm === r) return { root, path: '' }
    if (norm.startsWith(r + '/')) {
      return { root, path: norm.slice(r.length + 1) }
    }
  }
  return null
}

/** Pending go-to / diagnostic reveal applied when the target tab's model is ready. */
let pendingReveal: {
  root: string
  path: string
  line: number
  column: number
} | null = null

/** Queue a 1-based line/column reveal for a coding tab (consumed on model ready). */
export function queueReveal(
  root: string,
  path: string,
  line: number,
  column: number,
): void {
  pendingReveal = { root, path, line, column }
}

/** If a reveal is queued for this tab, take it (once). */
export function consumeReveal(
  root: string,
  path: string,
): { line: number; column: number } | null {
  const p = pendingReveal
  if (!p) return null
  if (p.root !== root || p.path !== path) return null
  pendingReveal = null
  return { line: p.line, column: p.column }
}
