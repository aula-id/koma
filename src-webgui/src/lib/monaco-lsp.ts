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
import {
  consumeReveal,
  mintId,
  pathToUri,
  queueReveal,
  splitRootPath,
  trackCompletion,
  trackCompletionResolve,
  trackDefinition,
  trackDocumentSymbol,
  trackFileText,
  trackHover,
  trackReferences,
  uriToPath,
  type LspCompletionItem,
  type LspCompletionList,
  type LspDiagnostic,
  type LspDocumentSymbol,
  type LspHover,
  type LspLocation,
  type LspTextEdit,
} from './lsp-bridge'

export {
  consumeReveal,
  mintId,
  pathToUri,
  queueReveal,
  resolveLspCompletion,
  resolveLspCompletionResolve,
  resolveLspDefinition,
  resolveLspDocumentSymbol,
  resolveLspFileText,
  resolveLspHover,
  resolveLspReferences,
  splitRootPath,
  uriToPath,
  type LspCompletionItem,
  type LspCompletionList,
  type LspDiagnostic,
  type LspDocumentSymbol,
  type LspHover,
  type LspLocation,
  type LspTextEdit,
} from './lsp-bridge'

// Workspace-wide "N references" CodeLens is off by default — each lens is a
// full textDocument/references search and can freeze the machine on a large
// rust-analyzer / vtsls index. Hover / go-to / peek still work.
const CODELENS_ENABLED = false
// Cap unique files materialised for peek/go-to so a popular symbol cannot
// FileRead hundreds of buffers into Monaco.
const PEEK_FILE_CAP = 24

type ReqFn = (body: object) => void
type RootsFn = () => string[]
/** Open a coding tab at 0-based line/character (go-to only; not peek). */
type OpenLocationFn = (uri: string, line: number, character: number) => void

// ─── Pending didChange flush (completion / resolve must see current buffer) ──
// CodeEditorTab debounces LspDidChange; completion fires immediately. Register
// a flusher per open file so provideCompletionItems / resolve can push the
// latest text before the RPC.
const pendingDidChangeFlush = new Map<string, () => void>()

function didChangeFlushKey(root: string, path: string): string {
  return `${root.replace(/\\/g, '/')}\0${path.replace(/\\/g, '/')}`
}

/** Called from CodeEditorTab while a coding file is mounted. Pass null to clear. */
export function registerLspDidChangeFlusher(
  root: string,
  path: string,
  flush: (() => void) | null,
): void {
  const key = didChangeFlushKey(root, path)
  if (flush) pendingDidChangeFlush.set(key, flush)
  else pendingDidChangeFlush.delete(key)
}

/**
 * Run the registered flusher (if any) and yield one macrotask so the host
 * notify worker can coalesce/send didChange before completion/resolve RPC.
 */
export async function flushPendingLspDidChange(root: string, path: string): Promise<void> {
  const flush = pendingDidChangeFlush.get(didChangeFlushKey(root, path))
  if (!flush) return
  flush()
  // Host notify worker coalesces didChange on a short quiet window (~16ms) and
  // always flushes pending changes before other notify jobs. One frame is not
  // enough when the request pool races the notify thread; wait a tick past the
  // coalesce window so the server buffer is current.
  await new Promise<void>((r) => setTimeout(r, 20))
}

// ─── CodeLens reference-count cache ──────────────────────────────────────────
// Survives tab close/reopen so "N references" does not re-hit the language
// server for every open of an unchanged file. Keyed by root+path+content hash.
const LENS_KINDS = new Set([5, 6, 9, 10, 11, 12, 23]) // Class..Struct
// Cap lenses so CodeLens warm stays bounded. Each lens is a references RPC;
// with host lock released during wait, concurrency is safe but still costly.
// Prefer earlier document-order symbols (usually types/functions near top).
// Symbols past the cap simply omit the "N references" lens — hover/goto still work.
const LENS_MAX = 24
const LENS_REF_CONCURRENCY = 3

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
    const symP = trackDocumentSymbol(symId)
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
    // Bounded concurrency — never stampede the host mutex with 40 parallel refs.
    for (let i = 0; i < candidates.length; i += LENS_REF_CONCURRENCY) {
      const batch = candidates.slice(i, i + LENS_REF_CONCURRENCY)
      await Promise.all(
        batch.map(async (sym) => {
          const refId = mintId('rlen')
          const refP = trackReferences(refId)
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
    }
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
 * Safe to call often; no-ops when version already cached. Does not block the UI.
 * `versionId` is Monaco `ITextModel.getVersionId()` (O(1) identity).
 */
export function warmCodeLensCache(
  req: ReqFn,
  root: string,
  path: string,
  versionId: number,
): void {
  if (!CODELENS_ENABLED) return
  if (!root || !path) return
  const hash = String(versionId)
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

// ─── Provider registration (once) ────────────────────────────────────────────

let providersReady = false
let openerReady = false

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
    if (preferText != null) {
      const next = preferText.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
      existing.setEOL(monaco.editor.EndOfLineSequence.LF)
      if (existing.getValue(monaco.editor.EndOfLinePreference.LF) !== next) {
        // Tab content is authoritative when provided.
        existing.setValue(next)
        existing.setEOL(monaco.editor.EndOfLineSequence.LF)
      }
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
    const p = trackFileText(requestId)
    req({ r: 'FileRead', root: split.root, path: split.path, requestId })
    try {
      text = await p
    } catch {
      return null
    }
  }

  const uri = monaco.Uri.parse(uriStr)
  const normalized = (text ?? '').replace(/\r\n/g, '\n').replace(/\r/g, '\n')

  // Another concurrent ensure may have created it.
  const raced = monaco.editor.getModel(uri)
  if (raced) {
    raced.setEOL(monaco.editor.EndOfLineSequence.LF)
    if (raced.getValue(monaco.editor.EndOfLinePreference.LF) !== normalized) {
      raced.setValue(normalized)
      raced.setEOL(monaco.editor.EndOfLineSequence.LF)
    }
    stampModelPath(raced, split.root, split.path)
    return raced
  }

  const model = monaco.editor.createModel(normalized, langFromPath(split.path), uri)
  model.setEOL(monaco.editor.EndOfLineSequence.LF)
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
  // Dedupe URI loads. Cap unique files so a popular symbol cannot stampede FileRead.
  const seen = new Set<string>()
  for (const l of locations) {
    if (!seen.has(l.uri)) {
      if (seen.size >= PEEK_FILE_CAP) continue
      seen.add(l.uri)
      await ensureModelForUri(l.uri, req, getRoots)
    }
    if (monaco.editor.getModel(monaco.Uri.parse(l.uri))) {
      out.push(locationToMonaco(l))
    }
  }
  return out
}

function lspRangeToMonaco(r: {
  startLine: number
  startCharacter: number
  endLine: number
  endCharacter: number
}): monaco.IRange {
  return {
    startLineNumber: r.startLine + 1,
    startColumn: r.startCharacter + 1,
    endLineNumber: r.endLine + 1,
    endColumn: r.endCharacter + 1,
  }
}

function toMonacoCompletion(
  it: LspCompletionItem,
  defaultRange: monaco.IRange,
  index: number,
  loc?: { root: string; path: string },
): monaco.languages.CompletionItem {
  const range = it.textEdit ? lspRangeToMonaco(it.textEdit.range) : defaultRange
  const insertText = it.textEdit?.newText ?? it.insertText ?? it.label
  const isSnippet = it.insertTextFormat === 2
  const additionalTextEdits = (it.additionalTextEdits ?? []).map((e) => ({
    range: lspRangeToMonaco(e.range),
    text: e.newText,
  }))
  const detail = it.detail || it.labelDescription
  const label: string | monaco.languages.CompletionItemLabel =
    it.labelDescription
      ? { label: it.label, description: it.labelDescription }
      : it.label
  return {
    label,
    kind: lspCompletionKind(it.kind),
    detail,
    documentation: it.documentation,
    insertText,
    insertTextRules: isSnippet
      ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
      : undefined,
    range,
    sortText: it.sortText ?? String(index).padStart(5, '0'),
    filterText: it.filterText,
    additionalTextEdits: additionalTextEdits.length ? additionalTextEdits : undefined,
    commitCharacters: it.commitCharacters,
    preselect: it.preselect,
    ...({ __koma: it, __loc: loc } as object),
  } as monaco.languages.CompletionItem
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

  // Union of common server trigger chars. Per-server caps aren't stored yet;
  // this set covers rust-analyzer / tsserver / clangd / gopls without waiting
  // on initialize plumbing. Spurious RPCs for rare chars are cheap vs missing
  // path completion after `::`.
  monaco.languages.registerCompletionItemProvider(selector, {
    triggerCharacters: ['.', ':', '<', '>', '"', "'", '/', '@', '(', '#', '`', '\\'],
    provideCompletionItems: async (model, position, context) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return { suggestions: [] }
      // Flush pending didChange so the server sees the buffer Monaco is completing on.
      // Without this, 500ms-debounced LspDidChange races completion and breaks
      // path completion + resolve hash matching (auto-import).
      await flushPendingLspDidChange(loc.root, loc.path)
      const requestId = mintId('cmp')
      const p = trackCompletion(requestId)
      // Monaco CompletionTriggerKind: Invoke=0, TriggerCharacter=1, TriggerForIncompleteCompletions=2
      // LSP: Invoked=1, TriggerCharacter=2, TriggerForIncompleteCompletions=3
      let triggerKind = 1
      let triggerCharacter: string | undefined
      if (context.triggerKind === monaco.languages.CompletionTriggerKind.TriggerCharacter) {
        triggerKind = 2
        triggerCharacter = context.triggerCharacter || undefined
      } else if (
        context.triggerKind === monaco.languages.CompletionTriggerKind.TriggerForIncompleteCompletions
      ) {
        triggerKind = 3
      }
      req({
        r: 'LspCompletion',
        root: loc.root,
        path: loc.path,
        line: position.lineNumber - 1,
        character: position.column - 1,
        triggerKind,
        triggerCharacter,
        requestId,
      })
      try {
        const list = await p
        const word = model.getWordUntilPosition(position)
        const defaultRange = {
          startLineNumber: position.lineNumber,
          startColumn: word?.startColumn ?? position.column,
          endLineNumber: position.lineNumber,
          endColumn: word?.endColumn ?? position.column,
        }
        const suggestions: monaco.languages.CompletionItem[] = list.items.map((it, i) =>
          toMonacoCompletion(it, defaultRange, i, loc),
        )
        return { suggestions, incomplete: list.isIncomplete }
      } catch {
        return { suggestions: [] }
      }
    },
    resolveCompletionItem: async (item) => {
      const raw = (item as monaco.languages.CompletionItem & { __koma?: LspCompletionItem; __loc?: { root: string; path: string } })
      const src = raw.__koma
      const loc = raw.__loc
      if (!src || !loc) return item
      // Already resolved with import edits — skip the round-trip.
      if (src.additionalTextEdits && src.additionalTextEdits.length > 0) {
        return item
      }
      // No data token → server has nothing more to resolve.
      if (src.data == null) return item
      // Ensure buffer is current before resolve — rust-analyzer re-runs completion
      // at the stored position and hash-matches; stale text drops additionalTextEdits.
      await flushPendingLspDidChange(loc.root, loc.path)
      const requestId = mintId('cmpr')
      const p = trackCompletionResolve(requestId)
      req({
        r: 'LspCompletionResolve',
        root: loc.root,
        path: loc.path,
        item: src,
        requestId,
      })
      try {
        const resolved = await p
        const range =
          item.range && !('insert' in (item.range as object))
            ? (item.range as monaco.IRange)
            : {
                startLineNumber: 1,
                startColumn: 1,
                endLineNumber: 1,
                endColumn: 1,
              }
        const next = toMonacoCompletion(resolved, range as monaco.IRange, 0, loc)
        // Preserve Monaco's internal bookkeeping fields.
        return { ...item, ...next, __koma: resolved, __loc: loc } as monaco.languages.CompletionItem
      } catch {
        return item
      }
    },
  })

  monaco.languages.registerHoverProvider(selector, {
    provideHover: async (model, position) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return null
      const requestId = mintId('hov')
      const p = trackHover(requestId)
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
      const p = trackDefinition(requestId)
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
      const p = trackReferences(requestId)
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

  // VS Code-style "N references" CodeLens above symbols — off by default.
  if (!CODELENS_ENABLED) return
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
      const model = ed.getModel()
      if (!model) return
      const pos = { lineNumber: payload.line, column: payload.column }
      ed.setPosition(pos)
      ed.revealPositionInCenterIfOutsideViewport(pos)
      ed.focus()

      const openPeek = () => {
        // Prefer stock Peek References (openInPeek: true even for 1 hit).
        // Never fall back to goToReferences — that jumps on single results.
        const peek =
          ed.getAction('editor.action.referenceSearch.trigger')
        if (peek) {
          void peek.run()
          return
        }
        // Contrib missing — build locations ourselves and force peek via goToLocations.
        void (async () => {
          const loc = modelToRootPath(model, getRoots())
          if (!loc) return
          const requestId = mintId('refclick')
          const p = trackReferences(requestId)
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
            await forcePeekReferences(ed, model.uri, pos, mapped)
          } catch {
            /* ignore */
          }
        })()
      }
      window.setTimeout(openPeek, 0)
    },
  )

  monaco.languages.registerCodeLensProvider(selector, {
    provideCodeLenses: async (model) => {
      const loc = modelToRootPath(model, getRoots())
      if (!loc) return { lenses: [], dispose: () => {} }

      // Prefer Monaco version id (O(1)) over hashing the full buffer twice.
      const hash = String(model.getVersionId())
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
        if (model.isDisposed() || String(model.getVersionId()) !== hash) {
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
 * swapping the current editor model.
 *
 * Peek's embedded preview editor is an EmbeddedCodeEditorWidget — we must NOT
 * hijack those opens or the peek widget never paints its preview and selection
 * can look like a jump.
 */
export function ensureEditorOpener(req: ReqFn, getRoots: RootsFn): void {
  if (openerReady) return
  openerReady = true

  monaco.editor.registerEditorOpener({
    openCodeEditor(source, resource, selectionOrPosition) {
      // Peek / embedded editors handle their own navigation.
      if (isEmbeddedEditor(source)) {
        return false
      }

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

      const open = goToHook
      if (open) {
        open(uriStr, line, character)
        return true
      }
      return false
    },
  })
}

/** Detect Monaco EmbeddedCodeEditorWidget (peek preview) without importing its type. */
function isEmbeddedEditor(ed: monaco.editor.ICodeEditor): boolean {
  const anyEd = ed as unknown as { getParentEditor?: () => unknown }
  return typeof anyEd.getParentEditor === 'function'
}

/**
 * Force the references peek widget open for the given locations (even if only one).
 * Uses Monaco's `editor.action.goToLocations` with openInPeek=true.
 */
async function forcePeekReferences(
  ed: monaco.editor.ICodeEditor,
  uri: monaco.Uri,
  pos: { lineNumber: number; column: number },
  locations: monaco.languages.Location[],
): Promise<void> {
  try {
    const { StandaloneServices } = await import(
      'monaco-editor/esm/vs/editor/standalone/browser/standaloneServices.js'
    )
    const { ICommandService } = await import(
      'monaco-editor/esm/vs/platform/commands/common/commands.js'
    )
    const cmd = StandaloneServices.get(ICommandService) as {
      executeCommand: (...args: unknown[]) => Promise<unknown>
    }
    await cmd.executeCommand(
      'editor.action.goToLocations',
      uri,
      pos,
      locations,
      'peek',
      'No references',
      true, // openInPeek — always peek, never jump
    )
    ed.focus()
  } catch {
    // Last resort: still do not jump — run stock peek action if it appeared late.
    void ed.getAction('editor.action.referenceSearch.trigger')?.run()
  }
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
    case 'php':
    case 'phtml':
    case 'php3':
    case 'php4':
    case 'php5':
    case 'phps':
      return 'php'
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
