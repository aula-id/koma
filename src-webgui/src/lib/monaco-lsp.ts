// Monaco ↔ host LSP bridge: pending request map + provider registration +
// diagnostic markers. Providers talk JSON-RPC through GuiReq; replies land as
// PushEnvelope variants handled in the koma store, which resolves the matching
// Promise here.

import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { langFromPath } from './monaco-setup'

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

export type LspCompletionItem = {
  label: string
  kind?: number
  detail?: string
  insertText?: string
  documentation?: string
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

type Pending<T> = {
  resolve: (v: T) => void
  reject: (e: Error) => void
  timer: ReturnType<typeof setTimeout>
}

const PENDING_MS = 12_000

const pendingCompletion = new Map<string, Pending<LspCompletionItem[]>>()
const pendingHover = new Map<string, Pending<LspHover | null>>()
const pendingDefinition = new Map<string, Pending<LspLocation[]>>()

let reqCounter = 0
function mintId(prefix: string): string {
  reqCounter += 1
  return `${prefix}-${Date.now().toString(36)}-${reqCounter}`
}

function track<T>(
  map: Map<string, Pending<T>>,
  id: string,
): Promise<T> {
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

/** Called from the store push reducer. */
export function resolveLspCompletion(
  requestId: string,
  items: LspCompletionItem[],
  error: string | null,
): void {
  settle(pendingCompletion, requestId, items ?? [], error)
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

// ─── URI helpers ─────────────────────────────────────────────────────────────

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

// ─── Markers ─────────────────────────────────────────────────────────────────

const MARKER_OWNER = 'koma-lsp'

export function applyDiagnosticsToMonaco(uri: string, diagnostics: LspDiagnostic[]): void {
  const models = monaco.editor.getModels()
  // Match by path suffix — Monaco models may use inmemory: or file:// URIs.
  const abs = uriToPath(uri)
  const model = models.find((m) => {
    const mu = m.uri.toString()
    if (mu === uri) return true
    if (abs && (mu.endsWith(abs) || mu.includes(abs))) return true
    return false
  })
  if (!model) {
    // No open model — Problems drawer still has the list; markers apply on next open.
    return
  }
  const markers: monaco.editor.IMarkerData[] = diagnostics.map((d) => ({
    severity: lspSeverityToMonaco(d.severity),
    message: d.message,
    startLineNumber: d.line + 1,
    startColumn: d.character + 1,
    endLineNumber: (d.endLine ?? d.line) + 1,
    endColumn: (d.endCharacter ?? d.character) + 1,
    source: d.source,
    code: d.code,
  }))
  monaco.editor.setModelMarkers(model, MARKER_OWNER, markers)
}

function lspSeverityToMonaco(s: number): monaco.MarkerSeverity {
  // 1 Error, 2 Warning, 3 Info, 4 Hint
  if (s === 1) return monaco.MarkerSeverity.Error
  if (s === 2) return monaco.MarkerSeverity.Warning
  if (s === 3) return monaco.MarkerSeverity.Info
  return monaco.MarkerSeverity.Hint
}

// ─── Provider registration (once) ────────────────────────────────────────────

let providersReady = false

type ReqFn = (body: object) => void
type RootsFn = () => string[]

type OpenLocationFn = (uri: string, line: number, character: number) => void

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

/**
 * Register completion / hover / definition providers for all languages.
 * Safe to call multiple times; only the first registration sticks.
 *
 * Go-to-definition cannot use Monaco's default file:// model service — our
 * editors use per-tab in-memory models. `openLocation` must open a coding tab
 * (and queue a line reveal) for cross-file targets.
 */
export function ensureLspProviders(
  req: ReqFn,
  getRoots: RootsFn,
  openLocation?: OpenLocationFn,
): void {
  if (providersReady) return
  providersReady = true

  const selector: monaco.languages.LanguageSelector = { language: '*' }

  monaco.languages.registerCompletionItemProvider(selector, {
    triggerCharacters: ['.', ':', '<', '"', "'", '/', '@'],
    provideCompletionItems: async (model, position) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return { suggestions: [] }
      const requestId = mintId('cmp')
      const p = track(pendingCompletion, requestId)
      req({
        r: 'LspCompletion',
        root: loc.root,
        path: loc.path,
        line: position.lineNumber - 1,
        character: position.column - 1,
        requestId,
      })
      try {
        const items = await p
        const suggestions: monaco.languages.CompletionItem[] = items.map((it, i) => {
          const range = {
            startLineNumber: position.lineNumber,
            startColumn: position.column,
            endLineNumber: position.lineNumber,
            endColumn: position.column,
          }
          // Expand range to word under cursor when possible.
          const word = model.getWordUntilPosition(position)
          if (word) {
            range.startColumn = word.startColumn
            range.endColumn = word.endColumn
          }
          return {
            label: it.label,
            kind: lspCompletionKind(it.kind),
            detail: it.detail,
            documentation: it.documentation,
            insertText: it.insertText ?? it.label,
            range,
            sortText: String(i).padStart(5, '0'),
          }
        })
        return { suggestions }
      } catch {
        return { suggestions: [] }
      }
    },
  })

  monaco.languages.registerHoverProvider(selector, {
    provideHover: async (model, position) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return null
      const requestId = mintId('hov')
      const p = track(pendingHover, requestId)
      req({
        r: 'LspHover',
        root: loc.root,
        path: loc.path,
        line: position.lineNumber - 1,
        character: position.column - 1,
        requestId,
      })
      try {
        const hover = await p
        if (!hover?.contents) return null
        const range = hover.range
          ? {
              startLineNumber: hover.range.startLine + 1,
              startColumn: hover.range.startCharacter + 1,
              endLineNumber: hover.range.endLine + 1,
              endColumn: hover.range.endCharacter + 1,
            }
          : undefined
        return {
          contents: [{ value: hover.contents }],
          range,
        }
      } catch {
        return null
      }
    },
  })

  monaco.languages.registerDefinitionProvider(selector, {
    provideDefinition: async (model, position) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return null
      const requestId = mintId('def')
      const p = track(pendingDefinition, requestId)
      req({
        r: 'LspDefinition',
        root: loc.root,
        path: loc.path,
        line: position.lineNumber - 1,
        character: position.column - 1,
        requestId,
      })
      try {
        const locations = await p
        if (!locations.length) return null
        const first = locations[0]
        // Compare absolute paths — model uses inmemory URI, LSP returns file://.
        const targetAbs = uriToPath(first.uri)
        const currentAbs = (() => {
          const a = loc.path
            ? `${loc.root.replace(/\/$/, '')}/${loc.path.replace(/^\//, '')}`
            : loc.root
          return a.replace(/\\/g, '/')
        })()
        const sameFile =
          !!targetAbs && targetAbs.replace(/\\/g, '/') === currentAbs

        // Always route through koma tabs — Monaco has no model for file:// URIs.
        if (openLocation) {
          openLocation(first.uri, first.range.startLine, first.range.startCharacter)
        }

        // Same file: return THIS model URI so Monaco can jump immediately.
        if (sameFile) {
          return {
            uri: model.uri,
            range: {
              startLineNumber: first.range.startLine + 1,
              startColumn: first.range.startCharacter + 1,
              endLineNumber: first.range.endLine + 1,
              endColumn: first.range.endCharacter + 1,
            },
          }
        }
        // Cross-file: tab open + reveal is handled by openLocation; returning a
        // file:// Location would no-op (no matching Monaco model).
        return null
      } catch {
        return null
      }
    },
  })
}

function modelToRootPath(
  model: monaco.editor.ITextModel,
  roots: string[],
): { root: string; path: string } | null {
  // Prefer metadata stamped by CodeEditorTab.
  const meta = (model as unknown as { __komaRoot?: string; __komaPath?: string })
  if (meta.__komaRoot && meta.__komaPath != null) {
    return { root: meta.__komaRoot, path: meta.__komaPath }
  }
  const u = model.uri.toString()
  const abs = uriToPath(u)
  if (!abs) return null
  return splitRootPath(abs, roots)
}

function lspCompletionKind(kind?: number): monaco.languages.CompletionItemKind {
  // LSP CompletionItemKind → Monaco (values mostly align 1..25).
  const K = monaco.languages.CompletionItemKind
  switch (kind) {
    case 1: return K.Text
    case 2: return K.Method
    case 3: return K.Function
    case 4: return K.Constructor
    case 5: return K.Field
    case 6: return K.Variable
    case 7: return K.Class
    case 8: return K.Interface
    case 9: return K.Module
    case 10: return K.Property
    case 11: return K.Unit
    case 12: return K.Value
    case 13: return K.Enum
    case 14: return K.Keyword
    case 15: return K.Snippet
    case 16: return K.Color
    case 17: return K.File
    case 18: return K.Reference
    case 19: return K.Folder
    case 20: return K.EnumMember
    case 21: return K.Constant
    case 22: return K.Struct
    case 23: return K.Event
    case 24: return K.Operator
    case 25: return K.TypeParameter
    default: return K.Text
  }
}

/** Stamp root/path on a model so providers can reverse-map without URI hacks. */
export function stampModelPath(
  model: monaco.editor.ITextModel,
  root: string,
  path: string,
): void {
  const m = model as unknown as { __komaRoot?: string; __komaPath?: string }
  m.__komaRoot = root
  m.__komaPath = path
}

export function languageIdForPath(path: string): string {
  // Prefer Monaco's mapped id; host accepts the same strings.
  return langFromPath(path)
}
