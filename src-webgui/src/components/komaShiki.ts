import { createHighlighterCore, type HighlighterCore } from 'shiki/core'
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript'
import type { BundledLanguage } from 'shiki'
import type { CodeHighlighterPlugin, HighlightOptions, HighlightResult } from '@streamdown/code'
import { luminance } from '../lib/luminance'

// Trimmed Shiki highlighter for the chat code blocks. `@streamdown/code`'s
// stock `code` plugin imports shiki's FULL `bundledLanguages` map, which makes
// Vite emit ~300 grammar chunks (~11MB dist) — unacceptable weight to bundle
// and serve over the offline `koma://` protocol. This is a drop-in
// CodeHighlighterPlugin (same contract Streamdown consumes) that ships only
// the ~16 languages we actually render, one theme, and Shiki's JavaScript
// regex engine (NO oniguruma `.wasm` blob to serve through the custom
// protocol). Everything is fine-grained dynamic imports, so each grammar is
// its own small lazy chunk fetched the first time that language appears.

// canonical shiki id -> lazy grammar import. Add a row to support a language.
const LANGS: Record<string, () => Promise<unknown>> = {
  typescript: () => import('@shikijs/langs/typescript'),
  tsx: () => import('@shikijs/langs/tsx'),
  javascript: () => import('@shikijs/langs/javascript'),
  jsx: () => import('@shikijs/langs/jsx'),
  python: () => import('@shikijs/langs/python'),
  rust: () => import('@shikijs/langs/rust'),
  go: () => import('@shikijs/langs/go'),
  bash: () => import('@shikijs/langs/bash'),
  json: () => import('@shikijs/langs/json'),
  yaml: () => import('@shikijs/langs/yaml'),
  toml: () => import('@shikijs/langs/toml'),
  sql: () => import('@shikijs/langs/sql'),
  markdown: () => import('@shikijs/langs/markdown'),
  diff: () => import('@shikijs/langs/diff'),
  html: () => import('@shikijs/langs/html'),
  css: () => import('@shikijs/langs/css'),
}

// Common fence aliases -> canonical id.
const ALIASES: Record<string, string> = {
  ts: 'typescript',
  js: 'javascript',
  py: 'python',
  rs: 'rust',
  sh: 'bash',
  shell: 'bash',
  zsh: 'bash',
  console: 'bash',
  yml: 'yaml',
  md: 'markdown',
  golang: 'go',
  htm: 'html',
}

// Both Shiki themes are loaded upfront (small, text-mate-color JSON, no
// grammars) so `activeShikiTheme()` can flip between them purely off the live
// koma palette at tokenize time — no lazy-load stall on a theme switch.
const THEMES = ['github-dark', 'github-light'] as const
type ShikiThemeName = (typeof THEMES)[number]

function normalizeLang(lang: string): string {
  const t = lang.trim().toLowerCase()
  return ALIASES[t] ?? t
}

// Picks the Shiki theme whose tuning matches the ACTIVE koma background —
// never OS prefers-color-scheme. Mirrors DiffTab.tsx's Monaco base pick
// (same `luminance()` helper, same >= 0.5 = light threshold), reading the raw
// hex `applyPaletteVars` (store/koma.ts) writes straight to `--koma-bg` on
// the document root (no color-mix probe needed — the var already holds a
// plain `#rrggbb`).
function activeShikiTheme(): ShikiThemeName {
  try {
    const bg = getComputedStyle(document.documentElement).getPropertyValue('--koma-bg').trim()
    if (!/^#[0-9a-fA-F]{6}$/.test(bg)) return 'github-dark'
    return luminance(bg) >= 0.5 ? 'github-light' : 'github-dark'
  } catch {
    return 'github-dark'
  }
}

const engine = createJavaScriptRegexEngine({ forgiving: true })
const loaded = new Set<string>()
let highlighterPromise: Promise<HighlighterCore> | null = null

function getHighlighter(): Promise<HighlighterCore> {
  if (highlighterPromise === null) {
    highlighterPromise = createHighlighterCore({
      themes: [import('@shikijs/themes/github-dark'), import('@shikijs/themes/github-light')],
      langs: [],
      engine,
    })
  }
  return highlighterPromise
}

// Ensure a grammar is registered on the singleton highlighter; resolves to the
// id actually usable ("text" when unknown/unsupported).
async function ensureLang(id: string): Promise<string> {
  if (!(id in LANGS)) return 'text'
  if (loaded.has(id)) return id
  const hl = await getHighlighter()
  await hl.loadLanguage(LANGS[id]() as never)
  loaded.add(id)
  return id
}

// Result cache keyed by theme + content head/tail + length + language (mirrors
// the stock plugin's cache so repeated frames of the same completed block
// don't re-tokenize). Theme is part of the key so flipping the koma palette
// light<->dark doesn't serve back stale-colored tokens for a block already
// cached under the other theme.
function cacheKey(code: string, lang: string, theme: string): string {
  const head = code.slice(0, 100)
  const tail = code.length > 100 ? code.slice(-100) : ''
  return `${theme}:${lang}:${code.length}:${head}:${tail}`
}

const resultCache = new Map<string, HighlightResult>()
const pendingCallbacks = new Map<string, Set<(r: HighlightResult) => void>>()

export const komaCode: CodeHighlighterPlugin = {
  name: 'shiki',
  type: 'code-highlighter',
  // Called fresh (not memoized) each time Streamdown recomputes its shiki
  // context value, which happens whenever MessageBody's `shikiTheme` prop
  // reference changes — see MessageBody.tsx. Returning the CURRENT
  // koma-appropriate theme in both slots is what makes that recompute pick
  // up a real (non-stale) value and cascade into a re-highlight.
  getThemes: () => {
    const theme = activeShikiTheme()
    return [theme, theme]
  },
  getSupportedLanguages: () => Object.keys(LANGS) as BundledLanguage[],
  supportsLanguage: (language: string) => normalizeLang(language) in LANGS,
  highlight(options: HighlightOptions, callback?: (result: HighlightResult) => void): HighlightResult | null {
    const id = normalizeLang(options.language as unknown as string)
    const theme = activeShikiTheme()
    const key = cacheKey(options.code, id, theme)
    const cached = resultCache.get(key)
    if (cached) return cached

    if (callback) {
      let set = pendingCallbacks.get(key)
      if (!set) {
        set = new Set()
        pendingCallbacks.set(key, set)
      }
      set.add(callback)
    }

    void (async () => {
      try {
        const usable = await ensureLang(id)
        const hl = await getHighlighter()
        const result = hl.codeToTokens(options.code, {
          lang: usable,
          // Feed the SAME koma-appropriate theme into both the light and
          // dark slots. Streamdown's rendered span only ever applies
          // `--sdm-c` (no separate `--shiki-dark` override) when light===dark,
          // so the token color is correct regardless of OS
          // prefers-color-scheme — driven purely by the koma palette.
          themes: { light: theme, dark: theme },
        }) as unknown as HighlightResult
        resultCache.set(key, result)
        const waiters = pendingCallbacks.get(key)
        if (waiters) {
          for (const cb of waiters) cb(result)
          pendingCallbacks.delete(key)
        }
      } catch (err) {
        // Highlighting failed (bad grammar / engine hiccup) — drop waiters so
        // the block just renders as un-highlighted plain text.
        console.error('[koma] code highlight failed:', err)
        pendingCallbacks.delete(key)
      }
    })()

    return null
  },
}
