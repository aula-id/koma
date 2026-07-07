import { createHighlighterCore, type HighlighterCore } from 'shiki/core'
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript'
import type { BundledLanguage } from 'shiki'
import type { CodeHighlighterPlugin, HighlightOptions, HighlightResult } from '@streamdown/code'

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

const THEME = 'github-dark' as const

function normalizeLang(lang: string): string {
  const t = lang.trim().toLowerCase()
  return ALIASES[t] ?? t
}

const engine = createJavaScriptRegexEngine({ forgiving: true })
const loaded = new Set<string>()
let highlighterPromise: Promise<HighlighterCore> | null = null

function getHighlighter(): Promise<HighlighterCore> {
  if (highlighterPromise === null) {
    highlighterPromise = createHighlighterCore({
      themes: [import('@shikijs/themes/github-dark')],
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

// Result cache keyed by content head/tail + length + language (mirrors the
// stock plugin's cache so repeated frames of the same completed block don't
// re-tokenize).
function cacheKey(code: string, lang: string): string {
  const head = code.slice(0, 100)
  const tail = code.length > 100 ? code.slice(-100) : ''
  return `${lang}:${code.length}:${head}:${tail}`
}

const resultCache = new Map<string, HighlightResult>()
const pendingCallbacks = new Map<string, Set<(r: HighlightResult) => void>>()

export const komaCode: CodeHighlighterPlugin = {
  name: 'shiki',
  type: 'code-highlighter',
  getThemes: () => [THEME, THEME],
  getSupportedLanguages: () => Object.keys(LANGS) as BundledLanguage[],
  supportsLanguage: (language: string) => normalizeLang(language) in LANGS,
  highlight(options: HighlightOptions, callback?: (result: HighlightResult) => void): HighlightResult | null {
    const id = normalizeLang(options.language as unknown as string)
    const key = cacheKey(options.code, id)
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
          themes: { light: THEME, dark: THEME },
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
