// Monaco ↔ host LSP bridge: pending request map + provider registration +
// diagnostic markers. Providers talk JSON-RPC through GuiReq; replies land as
// PushEnvelope variants handled in the koma store, which resolves the matching
// Promise here.
//
// Go-to / Peek:
//   Models use stable file:// URIs. Peek keeps an inline widget on those models.
//   Go-to (F12 / Ctrl+click) routes through registerEditorOpener → open coding tab.

import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
// Side-effect: load CodeLens + peek/goto contribs before providers register.
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

const pendingCompletion = new Map<string, Pending<LspCompletionItem[]>>()
const pendingHover = new Map<string, Pending<LspHover | null>>()
const pendingDefinition = new Map<string, Pending<LspLocation[]>>()
const pendingReferences = new Map<string, Pending<LspLocation[]>>()
const pendingDocumentSymbol = new Map<string, Pending<LspDocumentSymbol[]>>()
const pendingFileText = new Map<string, Pending<string>>()

// ─── CodeLens reference-count cache ──────────────────────────────────────────
// Survives tab close/reopen so "N references" does not re-hit the language
// server for every open of an unchanged file. Keyed by root+path+content hash.
const LENS_KINDS = new Set([5, 6, 9, 10, 11, 12, 23]) // Class..Struct
const LENS_MAX = 40

type CachedLens = {
  /** 0-based symbol range start line (decoration line). */
  rangeLine: number
  /** 0-based selection position used for references + peek. */
  selLine: number
  selCharacter: number
  count: number
}

type CodeLensCacheEntry = {
  hash: string
  lenses: CachedLens[]
}

const codeLensCache = new Map<string, CodeLensCacheEntry>()
/** In-flight compute promises so concurrent provideCodeLenses share one fetch. */
const codeLensInflight = new Map<string, Promise<CachedLens[]>>()

function fileCacheKey(root: string, path: string): string {
  return `${root.replace(/\\/g, '/')}\0${path.replace(/\\/g, '/')}`
}

/** Fast non-crypto fingerprint of file text (FNV-1a 32-bit + length). */
function contentHash(text: string): string {
  let h = 0x811c9dc5
  const n = text.length
  // Sample long files so open stays cheap; length + ends catch most edits.
  if (n <= 64_000) {
    for (let i = 0; i < n; i++) {
      h ^= text.charCodeAt(i)
      h = Math.imul(h, 0x01000193)
    }
  } else {
    const step = Math.floor(n / 32_000) || 1
    for (let i = 0; i < n; i += step) {
      h ^= text.charCodeAt(i)
      h = Math.imul(h, 0x01000193)
    }
    // Always mix head + tail.
    for (let i = 0; i < 256 && i < n; i++) {
      h ^= text.charCodeAt(i)
      h = Math.imul(h, 0x01000193)
    }
    for (let i = Math.max(0, n - 256); i < n; i++) {
      h ^= text.charCodeAt(i)
      h = Math.imul(h, 0x01000193)
    }
  }
  return `${n.toString(36)}:${(h >>> 0).toString(36)}`
}

function formatRefTitle(count: number): string {
  if (count === 0) return '0 references'
  if (count === 1) return '1 reference'
  return `${count} references`
}

function lensesFromCache(
  cached: CachedLens[],
  modelUri: string,
  commandId: string,
): monaco.languages.CodeLens[] {
  return cached.map((c) => ({
    range: {
      startLineNumber: c.rangeLine + 1,
      startColumn: 1,
      endLineNumber: c.rangeLine + 1,
      endColumn: 1,
    },
    command: {
      id: commandId,
      title: formatRefTitle(c.count),
      arguments: [
        {
          uri: modelUri,
          line: c.selLine + 1,
          column: Math.max(1, c.selCharacter + 1),
        },
      ],
    },
  }))
}

async function computeCodeLensCounts(
  req: ReqFn,
  root: string,
  path: string,
  hash: string,
): Promise<CachedLens[]> {
  const cacheKey = fileCacheKey(root, path)
  const hit = codeLensCache.get(cacheKey)
  if (hit && hit.hash === hash) return hit.lenses

  const inflightKey = `${cacheKey}\0${hash}`
  const existing = codeLensInflight.get(inflightKey)
  if (existing) return existing

  const work = (async () => {
    const symId = mintId('sym')
    const symP = track(pendingDocumentSymbol, symId)
    req({
      r: 'LspDocumentSymbol',
      root,
      path,
      requestId: symId,
    })
    let symbols: LspDocumentSymbol[] = []
    try {
      symbols = await symP
    } catch {
      return []
    }

    const candidates = symbols.filter((s) => LENS_KINDS.has(s.kind)).slice(0, LENS_MAX)
    const out: CachedLens[] = []
    await Promise.all(
      candidates.map(async (sym) => {
        const refId = mintId('rlen')
        const refP = track(pendingReferences, refId)
        req({
          r: 'LspReferences',
          root,
          path,
          line: sym.selectionRange.startLine,
          character: sym.selectionRange.startCharacter,
          includeDeclaration: false,
          requestId: refId,
        })
        let count = 0
        try {
          const refs = await refP
          count = refs.length
        } catch {
          return
        }
        out.push({
          rangeLine: sym.range.startLine,
          selLine: sym.selectionRange.startLine,
          selCharacter: sym.selectionRange.startCharacter,
          count,
        })
      }),
    )
    out.sort((a, b) => a.rangeLine - b.rangeLine || a.selLine - b.selLine)
    // Don't poison the cache when every references RPC failed (server still
    // starting / indexing) — leave miss so the next provide retries.
    if (candidates.length > 0 && out.length === 0) {
      return out
    }
    codeLensCache.set(cacheKey, { hash, lenses: out })
    return out
  })()

  codeLensInflight.set(inflightKey, work)
  try {
    return await work
  } finally {
    codeLensInflight.delete(inflightKey)
  }
}

/**
 * Eagerly warm the "N references" CodeLens cache after didOpen / didChange.
 * Safe to call often; no-ops when hash already cached. Does not block the UI.
 */
export function warmCodeLensCache(req: ReqFn, root: string, path: string, text: string): void {
  if (!root || !path) return
  const hash = contentHash(text)
  const cacheKey = fileCacheKey(root, path)
  const hit = codeLensCache.get(cacheKey)
  if (hit && hit.hash === hash) return
  void computeCodeLensCounts(req, root, path, hash).catch(() => {
    /* ignore warm failures */
  })
}

/** Drop cached lenses for a file (content rewrite / dispose). */
export function invalidateCodeLensCache(root: string, path: string): void {
  codeLensCache.delete(fileCacheKey(root, path))
}

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

/**
 * Resolve a FileRead that was issued for peek model materialization.
 * Store still applies reduceFileRead; this only settles the bridge Promise.
 */
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

export function monacoUriFromPath(root: string, path: string): monaco.Uri {
  return monaco.Uri.parse(pathToUri(root, path))
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

// ─── Pending go-to reveal (tab open path) ────────────────────────────────────

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

// ─── Provider registration (once) ────────────────────────────────────────────

let providersReady = false
let openerReady = false

type ReqFn = (body: object) => void
type RootsFn = () => string[]
/** Open a coding tab at 0-based line/character (go-to only; not peek). */
type OpenLocationFn = (uri: string, line: number, character: number) => void

/**
 * Get-or-create a Monaco text model for a workspace file URI.
 * Used by peek widgets and by the coding tab (stable file:// identity).
 */
export async function ensureModelForUri(
  uriStr: string,
  req: ReqFn,
  getRoots: RootsFn,
  preferText?: string | null,
): Promise<monaco.editor.ITextModel | null> {
  const existing = monaco.editor.getModel(monaco.Uri.parse(uriStr))
  if (existing) {
    if (preferText != null && existing.getValue() !== preferText) {
      // Tab content is authoritative when provided.
      existing.setValue(preferText)
    }
    return existing
  }

  const abs = uriToPath(uriStr)
  if (!abs) return null
  const roots = getRoots()
  const split = splitRootPath(abs, roots)
  if (!split) return null

  let text = preferText ?? null
  if (text == null) {
    const requestId = mintId('ftxt')
    const p = track(pendingFileText, requestId)
    req({ r: 'FileRead', root: split.root, path: split.path, requestId })
    try {
      text = await p
    } catch {
      return null
    }
  }

  const uri = monaco.Uri.parse(uriStr)
  // Another concurrent ensure may have created it.
  const raced = monaco.editor.getModel(uri)
  if (raced) {
    if (raced.getValue() !== text) raced.setValue(text)
    stampModelPath(raced, split.root, split.path)
    return raced
  }

  const model = monaco.editor.createModel(text, langFromPath(split.path), uri)
  stampModelPath(model, split.root, split.path)
  return model
}

function locationToMonaco(l: LspLocation): monaco.languages.Location {
  return {
    uri: monaco.Uri.parse(l.uri),
    range: {
      startLineNumber: l.range.startLine + 1,
      startColumn: l.range.startCharacter + 1,
      endLineNumber: l.range.endLine + 1,
      endColumn: l.range.endCharacter + 1,
    },
  }
}

async function materializeLocations(
  locations: LspLocation[],
  req: ReqFn,
  getRoots: RootsFn,
): Promise<monaco.languages.Location[]> {
  const out: monaco.languages.Location[] = []
  // Dedupe URI loads.
  const seen = new Set<string>()
  for (const l of locations) {
    if (!seen.has(l.uri)) {
      seen.add(l.uri)
      await ensureModelForUri(l.uri, req, getRoots)
    }
    if (monaco.editor.getModel(monaco.Uri.parse(l.uri))) {
      out.push(locationToMonaco(l))
    }
  }
  return out
}

/**
 * Register completion / hover / definition / references / CodeLens providers.
 * Safe to call multiple times; only the first registration sticks.
 *
 * Definition returns real file:// Locations so Peek Definition stays inline.
 * Go-to opens a coding tab via `registerEditorOpener` (see ensureEditorOpener).
 */
export function ensureLspProviders(
  req: ReqFn,
  getRoots: RootsFn,
  _openLocation?: OpenLocationFn,
): void {
  if (providersReady) return
  providersReady = true

  // Ctrl/Cmd+click go-to must NOT open peek (VS Code default).
  // multiCursorModifier is 'alt' so Ctrl/Cmd is free for definition links.
  ensureEditorOpener(req, getRoots)

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
        // Materialize target models so Peek can show content inline, and so
        // Go-to's editor opener can resolve the URI.
        const mapped = await materializeLocations(locations, req, getRoots)
        return mapped.length ? mapped : null
      } catch {
        return null
      }
    },
  })

  monaco.languages.registerReferenceProvider(selector, {
    provideReferences: async (model, position, context) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return null
      const requestId = mintId('ref')
      const p = track(pendingReferences, requestId)
      req({
        r: 'LspReferences',
        root: loc.root,
        path: loc.path,
        line: position.lineNumber - 1,
        character: position.column - 1,
        includeDeclaration: !!context.includeDeclaration,
        requestId,
      })
      try {
        const locations = await p
        if (!locations.length) return null
        const mapped = await materializeLocations(locations, req, getRoots)
        return mapped.length ? mapped : null
      } catch {
        return null
      }
    },
  })

  // VS Code-style "N references" CodeLens above symbols.
  // Click peeks references at the symbol (requires edcore.main contribs).
  const CODELENS_PEEK_REFS = 'koma.codelens.peekReferences'
  monaco.editor.registerCommand(
    CODELENS_PEEK_REFS,
    (_accessor, ...args: unknown[]) => {
      const payload = args[0] as
        | { uri?: string; line?: number; column?: number }
        | undefined
      if (!payload?.uri || payload.line == null || payload.column == null) return
      const editors = monaco.editor.getEditors()
      const ed =
        editors.find((e) => e.getModel()?.uri.toString() === payload.uri) ??
        editors.find((e) => e.hasTextFocus()) ??
        editors[0]
      if (!ed) return
      const pos = { lineNumber: payload.line, column: payload.column }
      ed.setPosition(pos)
      ed.revealPositionInCenterIfOutsideViewport(pos)
      ed.focus()
      // Defer one frame so the position sticks before the peek action reads it.
      const runPeek = () => {
        const action =
          ed.getAction('editor.action.referenceSearch.trigger') ??
          ed.getAction('editor.action.goToReferences')
        if (action) {
          void action.run()
          return
        }
        // Contribs missing — last resort: open first reference as a tab via provider.
        void (async () => {
          const model = ed.getModel()
          if (!model) return
          const loc = modelToRootPath(model, getRoots())
          if (!loc) return
          const requestId = mintId('refclick')
          const p = track(pendingReferences, requestId)
          req({
            r: 'LspReferences',
            root: loc.root,
            path: loc.path,
            line: payload.line! - 1,
            character: payload.column! - 1,
            includeDeclaration: false,
            requestId,
          })
          try {
            const locations = await p
            if (!locations.length) return
            const mapped = await materializeLocations(locations, req, getRoots)
            if (!mapped.length) return
            // Prefer multi-reference peek path through opener when action absent.
            const first = mapped[0]
            const uri = first.uri.toString()
            if (goToHook) {
              goToHook(
                uri,
                first.range.startLineNumber - 1,
                first.range.startColumn - 1,
              )
            }
          } catch {
            /* ignore */
          }
        })()
      }
      window.setTimeout(runPeek, 0)
    },
  )

  monaco.languages.registerCodeLensProvider(selector, {
    provideCodeLenses: async (model) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return { lenses: [], dispose: () => {} }

      const hash = contentHash(model.getValue())
      const cacheKey = fileCacheKey(loc.root, loc.path)
      const hit = codeLensCache.get(cacheKey)
      const modelUri = model.uri.toString()
      if (hit && hit.hash === hash) {
        return {
          lenses: lensesFromCache(hit.lenses, modelUri, CODELENS_PEEK_REFS),
          dispose: () => {},
        }
      }

      try {
        const cached = await computeCodeLensCounts(req, loc.root, loc.path, hash)
        // Model may have changed while we waited — only return if still matching.
        if (model.isDisposed() || contentHash(model.getValue()) !== hash) {
          return { lenses: [], dispose: () => {} }
        }
        return {
          lenses: lensesFromCache(cached, modelUri, CODELENS_PEEK_REFS),
          dispose: () => {},
        }
      } catch {
        return { lenses: [], dispose: () => {} }
      }
    },
  })
}

/**
 * Go-to-definition for cross-file targets: open a coding tab instead of
 * swapping the current editor model. Peek never calls this path.
 */
export function ensureEditorOpener(req: ReqFn, getRoots: RootsFn): void {
  if (openerReady) return
  openerReady = true

  monaco.editor.registerEditorOpener({
    openCodeEditor(source, resource, selectionOrPosition) {
      const uriStr = resource.toString()
      const abs = uriToPath(uriStr)
      if (!abs) return false

      // Same model already attached → let Monaco move the cursor.
      const srcModel = source.getModel()
      if (srcModel && srcModel.uri.toString() === uriStr) {
        return false
      }

      let line = 0
      let character = 0
      if (selectionOrPosition) {
        if ('startLineNumber' in selectionOrPosition) {
          line = Math.max(0, selectionOrPosition.startLineNumber - 1)
          character = Math.max(0, selectionOrPosition.startColumn - 1)
        } else if ('lineNumber' in selectionOrPosition) {
          line = Math.max(0, selectionOrPosition.lineNumber - 1)
          character = Math.max(0, selectionOrPosition.column - 1)
        }
      }

      // Ensure model exists (peek already did; go-to may race).
      void ensureModelForUri(uriStr, req, getRoots)

      // Dynamic import avoided — store is passed via openDiagnostic in ensureLspProviders call site.
      // Call through a global hook set by the store bootstrap.
      const open = goToHook
      if (open) {
        open(uriStr, line, character)
        return true
      }
      return false
    },
  })
}

/** Set by the store so the editor opener can open coding tabs without a cycle. */
let goToHook: OpenLocationFn | null = null

export function setGoToDefinitionHandler(fn: OpenLocationFn): void {
  goToHook = fn
}

function modelToRootPath(
  model: monaco.editor.ITextModel,
  roots: string[],
): { root: string; path: string } | null {
  const meta = model as unknown as { __komaRoot?: string; __komaPath?: string }
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
  // LSP languageId (not Monaco Monarch). Must match host language_id_for_path —
  // tsx/jsx need the React variants or vtsls typechecks them as plain TS/JS.
  const file = path.split(/[/\\]/).pop() ?? path
  const dot = file.lastIndexOf('.')
  const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
  switch (ext) {
    case 'rs':
      return 'rust'
    case 'ts':
    case 'mts':
    case 'cts':
      return 'typescript'
    case 'tsx':
      return 'typescriptreact'
    case 'js':
    case 'mjs':
    case 'cjs':
      return 'javascript'
    case 'jsx':
      return 'javascriptreact'
    case 'py':
    case 'pyi':
      return 'python'
    case 'go':
      return 'go'
    case 'c':
      return 'c'
    case 'h':
    case 'hpp':
    case 'hh':
    case 'cpp':
    case 'cc':
    case 'cxx':
      return 'cpp'
    case 'json':
    case 'jsonc':
      return 'json'
    case 'html':
    case 'htm':
    case 'xhtml':
      return 'html'
    case 'css':
      return 'css'
    case 'scss':
      return 'scss'
    case 'less':
      return 'less'
    case 'sh':
    case 'bash':
    case 'zsh':
      return 'shellscript'
    case 'toml':
      return 'toml'
    case 'lua':
      return 'lua'
    case 'zig':
    case 'zon':
      return 'zig'
    case 'nix':
      return 'nix'
    case 'md':
    case 'markdown':
      return 'markdown'
    case 'yaml':
    case 'yml':
      return 'yaml'
    default:
      return langFromPath(path)
  }
}

/** Drop a workspace file's Monaco model (discard / close-without-save). */
export function disposeCodingModel(root: string, path: string): void {
  const uri = monacoUriFromPath(root, path)
  const model = monaco.editor.getModel(uri)
  if (model) {
    monaco.editor.setModelMarkers(model, MARKER_OWNER, [])
    model.dispose()
  }
  // Keep CodeLens cache so reopen of unchanged content is instant.
}
