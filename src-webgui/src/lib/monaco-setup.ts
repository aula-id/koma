// Shared Monaco initialization — worker, languages, theme, font detection, language mapping.
// Used by both DiffTab and CodeEditorTab.

// Full editor core (CodeLens, peek references/definition, hover, suggest, find…).
// Must load BEFORE editor.api consumers create editors — lean editor.api alone
// registers no contrib actions, so getAction('editor.action.referenceSearch.trigger')
// is null and CodeLens clicks are no-ops.
import 'monaco-editor/esm/vs/editor/edcore.main.js'
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
import 'monaco-editor/esm/vs/basic-languages/php/php.contribution'
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
  php: 'php', phtml: 'php', php3: 'php', php4: 'php', php5: 'php', phps: 'php',
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
  const panel2 = resolveVarHex('--color-koma-panel2', panel)
  const fg = resolveVarHex('--color-koma-fg', '#c8d3f5')
  const dim = resolveVarHex('--color-koma-dim', '#adadad')
  const border = resolveVarHex('--color-koma-border', '#20242e')
  const hover = resolveVarHex('--color-koma-hover', '#1a1f2a')
  const accent = resolveVarHex('--color-koma-accent', '#39ff14')
  const warn = resolveVarHex('--color-koma-warn', '#ffb43c')
  const error = resolveVarHex('--color-koma-error', '#ff3c3c')
  const info = resolveVarHex('--color-koma-info', '#50c8ff')
  const success = resolveVarHex('--color-koma-success', '#00c853')
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
      'editorCursor.foreground': accent,
      'editor.selectionBackground': mixHex(accent, bg, 0.28),
      'editor.inactiveSelectionBackground': mixHex(accent, bg, 0.16),
      'editor.lineHighlightBackground': mixHex(fg, bg, 0.05),
      'editor.wordHighlightBackground': mixHex(accent, bg, 0.14),
      'editor.wordHighlightStrongBackground': mixHex(accent, bg, 0.22),
      'editor.findMatchBackground': mixHex(warn, bg, 0.35),
      'editor.findMatchHighlightBackground': mixHex(warn, bg, 0.18),
      'editor.findRangeHighlightBackground': mixHex(warn, bg, 0.12),
      'editorLink.activeForeground': accent,
      'editorCodeLens.foreground': dim,
      'editorIndentGuide.background1': mixHex(fg, bg, 0.08),
      'editorIndentGuide.activeBackground1': mixHex(fg, bg, 0.18),
      'editorBracketMatch.background': mixHex(accent, bg, 0.12),
      'editorBracketMatch.border': mixHex(accent, bg, 0.45),
      'editorInlayHint.foreground': dim,
      'editorInlayHint.background': mixHex(fg, bg, 0.06),
      'editorInlayHint.typeForeground': dim,
      'editorInlayHint.parameterForeground': dim,
      'editorStickyScroll.background': bg,
      'editorStickyScrollHover.background': hover,
      'editorStickyScroll.border': border,
      'editorStickyScroll.shadow': mixHex('#000000', bg, 0.35),
      'diffEditor.diagonalFill': border,
      'diffEditor.insertedTextBackground': mixHex(success, bg, 0.18),
      'diffEditor.removedTextBackground': mixHex(error, bg, 0.18),
      'diffEditor.insertedLineBackground': mixHex(success, bg, 0.10),
      'diffEditor.removedLineBackground': mixHex(error, bg, 0.10),
      'diffEditorGutter.insertedLineBackground': mixHex(success, bg, 0.22),
      'diffEditorGutter.removedLineBackground': mixHex(error, bg, 0.22),
      'editorWidget.background': panel,
      'editorWidget.foreground': fg,
      'editorWidget.border': border,
      'editorWidget.resizeBorder': accent,
      'editorOverviewRuler.border': border,
      'editorOverviewRuler.findMatchForeground': warn,
      'editorOverviewRuler.errorForeground': error,
      'editorOverviewRuler.warningForeground': warn,
      'editorOverviewRuler.infoForeground': info,
      'editorOverviewRuler.modifiedForeground': info,
      'editorOverviewRuler.addedForeground': success,
      'editorOverviewRuler.deletedForeground': error,
      'editorHoverWidget.background': panel,
      'editorHoverWidget.foreground': fg,
      'editorHoverWidget.border': border,
      'editorHoverWidget.statusBarBackground': panel2,
      'editorHoverWidget.highlightForeground': accent,
      'editorSuggestWidget.background': panel,
      'editorSuggestWidget.foreground': fg,
      'editorSuggestWidget.border': border,
      'editorSuggestWidget.selectedBackground': hover,
      'editorSuggestWidget.selectedForeground': fg,
      'editorSuggestWidget.selectedIconForeground': accent,
      'editorSuggestWidget.highlightForeground': accent,
      'editorSuggestWidget.focusHighlightForeground': accent,
      'editorSuggestWidgetStatus.foreground': dim,
      'editorMarkerNavigation.background': panel,
      'editorMarkerNavigationError.background': error,
      'editorMarkerNavigationWarning.background': warn,
      'editorMarkerNavigationInfo.background': info,
      'input.background': panel2,
      'input.foreground': fg,
      'input.border': border,
      'input.placeholderForeground': dim,
      'inputOption.activeBorder': accent,
      'inputOption.activeBackground': mixHex(accent, bg, 0.18),
      'inputOption.activeForeground': fg,
      'inputOption.hoverBackground': hover,
      'inputValidation.errorBackground': mixHex(error, bg, 0.25),
      'inputValidation.errorBorder': error,
      'inputValidation.errorForeground': fg,
      'inputValidation.warningBackground': mixHex(warn, bg, 0.25),
      'inputValidation.warningBorder': warn,
      'inputValidation.warningForeground': fg,
      'inputValidation.infoBackground': mixHex(info, bg, 0.25),
      'inputValidation.infoBorder': info,
      'inputValidation.infoForeground': fg,
      'list.activeSelectionBackground': hover,
      'list.activeSelectionForeground': fg,
      'list.inactiveSelectionBackground': mixHex(fg, bg, 0.08),
      'list.inactiveSelectionForeground': fg,
      'list.hoverBackground': mixHex(fg, bg, 0.06),
      'list.hoverForeground': fg,
      'list.focusBackground': hover,
      'list.focusForeground': fg,
      'list.highlightForeground': accent,
      'list.focusHighlightForeground': accent,
      'tree.indentGuidesStroke': border,
      'scrollbar.shadow': bg,
      'scrollbarSlider.background': mixHex(fg, bg, 0.18),
      'scrollbarSlider.hoverBackground': mixHex(fg, bg, 0.28),
      'scrollbarSlider.activeBackground': mixHex(fg, bg, 0.38),
      'widget.shadow': mixHex('#000000', bg, 0.45),
      'focusBorder': accent,
      'textLink.foreground': accent,
      'textLink.activeForeground': accent,
      'textCodeBlock.background': panel2,
      'textBlockQuote.background': panel2,
      'textBlockQuote.border': border,
      'textPreformat.foreground': fg,
      'textSeparator.foreground': border,
      'descriptionForeground': dim,
      'peekView.border': accent,
      'peekViewTitle.background': panel,
      'peekViewTitleLabel.foreground': fg,
      'peekViewTitleDescription.foreground': dim,
      'peekViewEditor.background': bg,
      'peekViewEditorGutter.background': bg,
      'peekViewEditor.matchHighlightBackground': mixHex(warn, bg, 0.28),
      'peekViewEditor.matchHighlightBorder': mixHex(warn, bg, 0.5),
      'peekViewResult.background': panel,
      'peekViewResult.fileForeground': fg,
      'peekViewResult.lineForeground': dim,
      'peekViewResult.matchHighlightBackground': mixHex(warn, bg, 0.28),
      'peekViewResult.selectionBackground': hover,
      'peekViewResult.selectionForeground': fg,
      'editorError.foreground': error,
      'editorWarning.foreground': warn,
      'editorInfo.foreground': info,
      'editorHint.foreground': info,
      'editorGutter.modifiedBackground': info,
      'editorGutter.addedBackground': success,
      'editorGutter.deletedBackground': error,
      'dropdown.background': panel,
      'dropdown.foreground': fg,
      'dropdown.border': border,
      'dropdown.listBackground': panel,
      'menu.background': panel,
      'menu.foreground': fg,
      'menu.border': border,
      'menu.selectionBackground': hover,
      'menu.selectionForeground': fg,
      'menu.separatorBackground': border,
      'toolbar.hoverBackground': hover,
      'toolbar.activeBackground': mixHex(fg, bg, 0.12),
      'button.background': mixHex(accent, bg, 0.22),
      'button.foreground': fg,
      'button.hoverBackground': mixHex(accent, bg, 0.32),
      'button.border': mixHex(accent, bg, 0.4),
      'button.secondaryBackground': panel2,
      'button.secondaryForeground': fg,
      'button.secondaryHoverBackground': hover,
      'keybindingLabel.background': panel2,
      'keybindingLabel.foreground': dim,
      'keybindingLabel.border': border,
      'keybindingLabel.bottomBorder': border,
      'badge.background': mixHex(accent, bg, 0.22),
      'badge.foreground': fg,
      'progressBar.background': accent,
    },
  })
  // Context menus mount on document.body (outside .monaco-editor). Monaco only
  // injects `--vscode-*` color vars under `.monaco-editor, .monaco-diff-editor,
  // .monaco-component`, so body menus keep the previous base theme (often dark
  // on a light koma palette). Mirror the menu tokens onto :root every apply.
  mirrorMenuCssVars({
    bg,
    panel,
    panel2,
    fg,
    dim,
    border,
    hover,
  })
  return name
}

/** Push menu-related --vscode-* tokens onto :root for body-mounted menus. */
function mirrorMenuCssVars(p: {
  bg: string
  panel: string
  panel2: string
  fg: string
  dim: string
  border: string
  hover: string
}): void {
  if (typeof document === 'undefined') return
  const root = document.documentElement
  const shadow = mixHex('#000000', p.bg, 0.45)
  const set = (name: string, value: string) => root.style.setProperty(name, value)
  // Hex (not var()) so inline styles like backgroundColor: var(--vscode-menu-background)
  // always resolve even when the menu is outside .monaco-editor.
  set('--vscode-menu-background', p.panel)
  set('--vscode-menu-foreground', p.fg)
  set('--vscode-menu-border', p.border)
  set('--vscode-menu-selectionBackground', p.hover)
  set('--vscode-menu-selectionForeground', p.fg)
  set('--vscode-menu-separatorBackground', p.border)
  set('--vscode-widget-shadow', shadow)
  set('--vscode-widget-border', p.border)
  set('--vscode-scrollbar-shadow', p.bg)
  set('--vscode-scrollbarSlider-background', mixHex(p.fg, p.bg, 0.18))
  set('--vscode-scrollbarSlider-hoverBackground', mixHex(p.fg, p.bg, 0.28))
  set('--vscode-scrollbarSlider-activeBackground', mixHex(p.fg, p.bg, 0.38))
  set('--vscode-keybindingLabel-background', p.panel2)
  set('--vscode-keybindingLabel-foreground', p.dim)
  set('--vscode-keybindingLabel-border', p.border)
  set('--vscode-keybindingLabel-bottomBorder', p.border)
  set('--vscode-focusBorder', resolveVarHex('--color-koma-accent', '#39ff14'))
  // menu.background defaults to selectBackground in Monaco's color registry —
  // keep both in lockstep for any path that still reads select.*.
  set('--vscode-select-background', p.panel)
  set('--vscode-select-foreground', p.fg)
  set('--vscode-select-border', p.border)
  set('--vscode-list-activeSelectionBackground', p.hover)
  set('--vscode-list-activeSelectionForeground', p.fg)
  set('--vscode-editorActionList-background', p.panel)
  set('--vscode-editorActionList-foreground', p.fg)
  set('--vscode-editorActionList-focusBackground', p.hover)
  set('--vscode-editorActionList-focusForeground', p.fg)
  set('--vscode-dropdown-background', p.panel)
  set('--vscode-dropdown-foreground', p.fg)
  set('--vscode-dropdown-border', p.border)
  set('--vscode-dropdown-listBackground', p.panel)
}

// Re-apply koma Monaco themes after a live palette change (Settings / Snapshot).
// Safe no-op if Monaco has never been initialized in this page load.
export function refreshKomaThemes(): void {
  if (typeof document === 'undefined') return
  try {
    // defineTheme is cheap; always refresh both named themes so open editors
    // and any floating widgets pick up the new colours on the next setTheme.
    applyKomaTheme('koma-editor')
    applyKomaTheme('koma-diff')
    // Re-assert the active theme so open widgets repaint. Prefer koma-editor;
    // DiffTab re-sets koma-diff on its own mount path.
    monaco.editor.setTheme('koma-editor')
  } catch {
    /* monaco not ready */
  }
}

// Blend `fg` over `bg` by `amount` (0..1) → hex. Used for translucent selection /
// hover colours Monaco can't take as CSS color-mix.
function mixHex(fg: string, bg: string, amount: number): string {
  const a = Math.max(0, Math.min(1, amount))
  const parse = (h: string): [number, number, number] | null => {
    const s = h.replace('#', '').trim()
    if (s.length === 3) {
      return [
        parseInt(s[0] + s[0], 16),
        parseInt(s[1] + s[1], 16),
        parseInt(s[2] + s[2], 16),
      ]
    }
    if (s.length >= 6) {
      return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16)]
    }
    return null
  }
  const A = parse(fg)
  const B = parse(bg)
  if (!A || !B) return fg
  const m = (x: number, y: number) => Math.round(x * a + y * (1 - a))
  const to = (n: number) => n.toString(16).padStart(2, '0')
  return `#${to(m(A[0], B[0]))}${to(m(A[1], B[1]))}${to(m(A[2], B[2]))}`
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
