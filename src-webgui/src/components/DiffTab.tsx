import { useEffect, useRef, type ReactNode } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker&inline'
import { Loader2 } from 'lucide-react'
import type { Tab } from '../store/koma'
import { luminance } from '../lib/luminance'

// ---- Monaco language contributions (Monarch tokenizers ONLY) ----------------
// editor.api ships ZERO languages. Each basic-languages `.contribution` import
// self-registers a MAIN-THREAD Monarch tokenizer (syntax colours) — NO language
// worker is pulled in (those live under monaco-editor/esm/vs/language/* and are
// deliberately omitted). The only worker is the inlined base editor worker
// below, used solely for diff computation.
import 'monaco-editor/esm/vs/basic-languages/typescript/typescript.contribution'
import 'monaco-editor/esm/vs/basic-languages/javascript/javascript.contribution'
import 'monaco-editor/esm/vs/basic-languages/css/css.contribution'
import 'monaco-editor/esm/vs/basic-languages/scss/scss.contribution'
import 'monaco-editor/esm/vs/basic-languages/less/less.contribution'
import 'monaco-editor/esm/vs/basic-languages/html/html.contribution'
import 'monaco-editor/esm/vs/basic-languages/xml/xml.contribution'
import 'monaco-editor/esm/vs/basic-languages/markdown/markdown.contribution'
import 'monaco-editor/esm/vs/basic-languages/rust/rust.contribution'
import 'monaco-editor/esm/vs/basic-languages/python/python.contribution'
import 'monaco-editor/esm/vs/basic-languages/go/go.contribution'
import 'monaco-editor/esm/vs/basic-languages/shell/shell.contribution'
import 'monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution'
import 'monaco-editor/esm/vs/basic-languages/sql/sql.contribution'
import 'monaco-editor/esm/vs/basic-languages/cpp/cpp.contribution'
import 'monaco-editor/esm/vs/basic-languages/java/java.contribution'
import 'monaco-editor/esm/vs/basic-languages/ini/ini.contribution'

// Custom-protocol (koma://) webviews can't reliably fetch module workers, so the
// base editor worker is inlined via vite's `?worker&inline` (compiled to a blob
// URL — no network). NO language workers are registered, so getWorker returns
// the one base worker regardless of the requested label.
;(self as unknown as {
  MonacoEnvironment?: { getWorker: (workerId: string, label: string) => Worker }
}).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
}

type DiffTabModel = Extract<Tab, { kind: 'diff' }>

// ext -> Monarch language id (all registered above). JSON has no basic-languages
// tokenizer (its highlighter ships with the worker-backed json service we omit),
// so it borrows the javascript tokenizer — close enough for coloured JSON diffs.
// Unknown extensions fall back to plaintext.
const EXT_LANG: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  json: 'javascript', jsonc: 'javascript',
  css: 'css', scss: 'scss', sass: 'scss', less: 'less',
  html: 'html', htm: 'html', xhtml: 'html', xml: 'xml', svg: 'xml',
  md: 'markdown', markdown: 'markdown', mdx: 'markdown',
  rs: 'rust',
  py: 'python', pyi: 'python',
  go: 'go',
  sh: 'shell', bash: 'shell', zsh: 'shell',
  yaml: 'yaml', yml: 'yaml',
  sql: 'sql',
  c: 'cpp', h: 'cpp', hpp: 'cpp', hh: 'cpp', cpp: 'cpp', cc: 'cpp', cxx: 'cpp',
  java: 'java',
  ini: 'ini', toml: 'ini', cfg: 'ini', conf: 'ini',
}

function langFromPath(path: string): string {
  const file = path.split('/').pop() ?? path
  const dot = file.lastIndexOf('.')
  const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
  return EXT_LANG[ext] ?? 'plaintext'
}

// ---- Live palette -> Monaco theme -------------------------------------------
// Resolve a CSS custom property (incl. color-mix() expressions like koma-panel/
// border) to a concrete hex. getComputedStyle on a raw custom property returns
// the UNRESOLVED expression, so we set it on a probe element's `color` and read
// the browser-computed rgb back, then convert to hex (Monaco wants hex).
function rgbToHex(rgb: string): string | null {
  const trimmed = rgb.trim()
  const m = trimmed.match(/rgba?\(([^)]+)\)/i)
  if (!m) return /^#[0-9a-f]{3,8}$/i.test(trimmed) ? trimmed : null
  const parts = m[1].split(/[ ,/]+/).map((s) => s.trim()).filter(Boolean)
  if (parts.length < 3) return null
  const toHex = (v: string) => {
    const n = Math.max(0, Math.min(255, Math.round(parseFloat(v))))
    return n.toString(16).padStart(2, '0')
  }
  return `#${toHex(parts[0])}${toHex(parts[1])}${toHex(parts[2])}`
}

function resolveVarHex(varName: string, fallback: string): string {
  try {
    const probe = document.createElement('span')
    probe.style.color = `var(${varName})`
    probe.style.position = 'absolute'
    probe.style.visibility = 'hidden'
    probe.style.pointerEvents = 'none'
    document.body.appendChild(probe)
    const rgb = getComputedStyle(probe).color
    document.body.removeChild(probe)
    return rgbToHex(rgb) ?? fallback
  } catch {
    return fallback
  }
}

const KOMA_THEME = 'koma-diff'

// (Re)define the Monaco theme from the live --color-koma-* palette. Called on
// each DiffTab mount so palette changes mostly track (perfect live re-theming of
// an already-open editor is a NON-GOAL — follow-up). base (vs/vs-dark) is picked
// by the canvas luminance, so the inherited default diff insert/remove tints
// (left as-is) read correctly over our background.
function applyKomaTheme(): string {
  const bg = resolveVarHex('--color-koma-bg', '#0b0e14')
  const panel = resolveVarHex('--color-koma-panel', '#151922')
  const fg = resolveVarHex('--color-koma-fg', '#c8d3f5')
  const dim = resolveVarHex('--color-koma-dim', '#adadad')
  const border = resolveVarHex('--color-koma-border', '#20242e')
  const base: monaco.editor.BuiltinTheme = luminance(bg) < 0.5 ? 'vs-dark' : 'vs'
  monaco.editor.defineTheme(KOMA_THEME, {
    base,
    inherit: true,
    rules: [],
    colors: {
      'editor.background': bg,
      'editor.foreground': fg,
      'editorLineNumber.foreground': dim,
      'editorLineNumber.activeForeground': fg,
      'editorGutter.background': bg,
      'diffEditor.diagonalFill': border,
      'editorWidget.background': panel,
      'editorWidget.border': border,
      'editorOverviewRuler.border': border,
    },
  })
  return KOMA_THEME
}

// The app's mono stack (KomaMono) — the same family the chat renders with.
function readMonoFont(): string {
  try {
    const f = getComputedStyle(document.body).fontFamily
    if (f && f.trim() !== '') return f
  } catch {
    /* ignore */
  }
  return "'KomaMono', ui-monospace, 'JetBrains Mono', monospace"
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full w-full items-center justify-center px-6 text-center text-[12px] text-koma-dim">
      {children}
    </div>
  )
}

// Side-by-side Monaco DiffEditor for one File-changed path. Lazy-loaded (this
// whole module + monaco lands in its own async chunk), so nothing here touches
// the main bundle until the first diff tab is opened.
export default function DiffTab({ tab }: { tab: DiffTabModel }) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null)
  const modelsRef = useRef<{
    original: monaco.editor.ITextModel
    modified: monaco.editor.ITextModel
  } | null>(null)

  const diff = tab.diff
  const showEditor = diff != null && (diff.error == null || diff.error === '') && !diff.binary

  // Create the diff editor once, when it first becomes showable; dispose on
  // unmount (or when it stops being showable, e.g. a re-request returns an
  // error/binary). NOT re-created on diff-content changes, so re-focus of an
  // already-open tab doesn't flash.
  useEffect(() => {
    const host = containerRef.current
    if (!showEditor || !host) return
    const theme = applyKomaTheme()
    const editor = monaco.editor.createDiffEditor(host, {
      renderSideBySide: true,
      readOnly: true,
      originalEditable: false,
      automaticLayout: true,
      minimap: { enabled: false },
      folding: false,
      scrollBeyondLastLine: false,
      fontFamily: readMonoFont(),
      fontSize: 12,
      lineNumbersMinChars: 3,
      theme,
    })
    monaco.editor.setTheme(theme)
    editorRef.current = editor
    return () => {
      editor.dispose()
      editorRef.current = null
    }
  }, [showEditor])

  // Sync the two models to the current diff payload — initial open AND every
  // re-request on re-activate. The editor persists across these; only the models
  // are swapped, so a re-focus updates content without a rebuild/flash. Monaco's
  // createDiffEditor.dispose() does NOT dispose caller-owned models, so we own
  // their lifecycle explicitly (dispose the prior pair after setting the new).
  useEffect(() => {
    if (!showEditor || !diff || !editorRef.current) return
    const lang = langFromPath(tab.path)
    const original = monaco.editor.createModel(diff.original, lang)
    const modified = monaco.editor.createModel(diff.modified, lang)
    editorRef.current.setModel({ original, modified })
    const prev = modelsRef.current
    modelsRef.current = { original, modified }
    if (prev) {
      prev.original.dispose()
      prev.modified.dispose()
    }
  }, [diff, showEditor, tab.path])

  // Drop whatever models remain on final unmount (tab closed / session switch).
  useEffect(
    () => () => {
      const m = modelsRef.current
      if (m) {
        m.original.dispose()
        m.modified.dispose()
        modelsRef.current = null
      }
    },
    [],
  )

  // ---- Body states ----------------------------------------------------------
  if (diff && diff.error != null && diff.error !== '')
    return (
      <div className="relative h-full w-full">
        <Centered>{diff.error}</Centered>
        {tab.loading && (
          <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
            <Loader2 size={14} className="animate-spin opacity-70" />
          </div>
        )}
      </div>
    )
  if (diff && diff.binary)
    return (
      <div className="relative h-full w-full">
        <Centered>binary file — no preview</Centered>
        {tab.loading && (
          <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
            <Loader2 size={14} className="animate-spin opacity-70" />
          </div>
        )}
      </div>
    )
  if (!diff) {
    return (
      <div className="flex h-full w-full items-center justify-center text-koma-dim">
        <Loader2 size={18} className="animate-spin opacity-70" />
      </div>
    )
  }
  return (
    <div className="relative h-full w-full">
      <div ref={containerRef} className="absolute inset-0" />
      {/* Non-git dirs diff against the session's first-touch pre-image ("virtual
          git") — badge the origin so nobody mistakes it for a git diff. */}
      {diff.origin === 'baseline' && (
        <div className="pointer-events-none absolute bottom-2 right-4 rounded border border-koma-border bg-koma-panel/90 px-1.5 py-0.5 font-mono text-[10px] text-koma-dim">
          session baseline
        </div>
      )}
      {tab.loading && (
        <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
          <Loader2 size={14} className="animate-spin opacity-70" />
        </div>
      )}
    </div>
  )
}
