// Shared Monaco initialization — worker, languages, theme, font detection, language mapping.
// Used by both DiffTab and CodeEditorTab.

import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker&inline'
import { luminance } from './luminance'

// Language contributions (Monarch tokenizers only)
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
let monacoReady = false

export function initMonaco(): void {
  if (monacoReady) return
  ;(self as unknown as {
    MonacoEnvironment?: { getWorker: (workerId: string, label: string) => Worker }
  }).MonacoEnvironment = {
    getWorker: () => new EditorWorker(),
  }
  monacoReady = true
}

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

export function langFromPath(path: string): string {
  const file = path.split('/').pop() ?? path
  const dot = file.lastIndexOf('.')
  const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
  return EXT_LANG[ext] ?? 'plaintext'
}

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

// (Re)define a Monaco theme from the live --color-koma-* palette. `name` defaults
// to the standalone code editor theme; DiffTab passes 'koma-diff'.
export function applyKomaTheme(name = 'koma-editor'): string {
  const bg = resolveVarHex('--color-koma-bg', '#0b0e14')
  const panel = resolveVarHex('--color-koma-panel', '#151922')
  const fg = resolveVarHex('--color-koma-fg', '#c8d3f5')
  const dim = resolveVarHex('--color-koma-dim', '#adadad')
  const border = resolveVarHex('--color-koma-border', '#20242e')
  const base: monaco.editor.BuiltinTheme = luminance(bg) < 0.5 ? 'vs-dark' : 'vs'
  monaco.editor.defineTheme(name, {
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
  return name
}

// The app's mono stack (KomaMono) — the same family the chat renders with.
export function readMonoFont(): string {
  try {
    const f = getComputedStyle(document.body).fontFamily
    if (f && f.trim() !== '') return f
  } catch {
    /* ignore */
  }
  return "'KomaMono', ui-monospace, 'JetBrains Mono', monospace"
}
