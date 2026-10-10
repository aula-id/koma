import manifest from '../../../src-misc/help/manifest.json'
import commandSource from '../../../src-agent/src/controller/command.rs?raw'
const documents = import.meta.glob('../../../src-misc/help/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>
export const HELP_MANIFEST = manifest
export function helpArticle(id: string): string {
  const topic = manifest.articles.find(a => a.id === id)
  return topic ? documents[`../../../src-misc/help/${topic.file}`] ?? '' : ''
}
export function rankHelp(query: string) {
  const q = query.toLowerCase()
  const wantTui = ['tui', 'terminal', 'slash', 'keybind', 'hotkey', '/help', '/settings', '/model'].some(k => q.includes(k))
  const terms = q.split(/[^\p{L}\p{N}]+/u).filter(t => t.length > 2)
  return manifest.articles.map((a, index) => {
    const title = `${a.title} ${a.aliases.join(' ')}`.toLowerCase()
    const body = helpArticle(a.id).toLowerCase()
    let score = terms.reduce((n, t) => n + (title.includes(t) ? 8 : body.includes(t) ? 1 : 0), 0)
    const isTui = a.id.startsWith('tui-')
    if (!wantTui && isTui) score = Math.floor(score / 4)
    else if (!wantTui && !isTui && score > 0) score += 2
    else if (wantTui && isTui && score > 0) score += 4
    return { ...a, score, index }
  }).filter(a => !terms.length || a.score > 0).sort((a, b) => b.score - a.score || a.index - b.index)
}
// Read the real registries at bundle time; no separately maintained command list.
export function helpRegistry(name: 'COMMANDS' | 'KEYBINDINGS') {
  const source = commandSource.split(`pub const ${name}:`)[1]?.split('];')[0] ?? ''
  return [...source.matchAll(/\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,?\s*\)/g)].map(m => ({ key: m[1], description: m[2] }))
}
/** Pull a JSON object out of model output that may include fences or prose. */
export function extractHelpJson(raw: string): string {
  let t = raw.trim()
  if (t.startsWith('```')) {
    t = t.replace(/^```(?:json|JSON)?\r?\n?/, '')
    t = t.split('```')[0]?.trim() ?? t
  }
  if (t.startsWith('{')) {
    const end = findTopLevelObjectEnd(t)
    return end >= 0 ? t.slice(0, end + 1).trim() : t
  }
  const start = t.indexOf('{')
  if (start < 0) throw new Error('Help returned invalid guidance. Retry or use Reference and Guides.')
  const slice = t.slice(start)
  const end = findTopLevelObjectEnd(slice)
  if (end < 0) throw new Error('Help returned invalid guidance. Retry or use Reference and Guides.')
  return slice.slice(0, end + 1).trim()
}
function findTopLevelObjectEnd(s: string): number {
  let depth = 0
  let inStr = false
  let escape = false
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]
    if (inStr) {
      if (escape) escape = false
      else if (ch === '\\') escape = true
      else if (ch === '"') inStr = false
      continue
    }
    if (ch === '"') inStr = true
    else if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) return i
    }
  }
  return -1
}
export function parseHelpAnswer(raw: string) {
  const v = JSON.parse(extractHelpJson(raw))
  if (!v || typeof v !== 'object') throw new Error('Help returned invalid guidance. Retry or use Reference and Guides.')
  const answer = v.answer
  const articles = Array.isArray(v.articles) ? v.articles : []
  const navigation = v.navigation === undefined ? undefined : v.navigation
  const guide = v.guide === undefined ? undefined : v.guide
  if ((navigation !== undefined && navigation !== null && typeof navigation !== 'string')
    || (guide !== undefined && guide !== null && typeof guide !== 'string')
    || typeof answer !== 'string'
    || !answer.trim()
    || articles.some((id: unknown) => !manifest.articles.some(a => a.id === id))
    || (navigation && !manifest.navigation.includes(navigation))
    || (guide && !manifest.workflows.includes(guide))) {
    throw new Error('Help returned invalid guidance. Retry or use Reference and Guides.')
  }
  return { answer, articles, navigation, guide } as { answer: string; articles: string[]; navigation?: string | null; guide?: string | null }
}

/** Rebuild the host-facing JSON for an assistant transcript turn. */
export function tutorialAssistantWire(m: { content: string; articles?: string[]; navigation?: string | null; tour?: string | null }) {
  return JSON.stringify({
    answer: m.content,
    articles: m.articles ?? [],
    navigation: m.navigation ?? null,
    guide: m.tour ?? null,
  })
}
