import manifest from '../../../src-misc/help/manifest.json'
import commandSource from '../../../src-agent/src/controller/command.rs?raw'
const documents = import.meta.glob('../../../src-misc/help/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>
export const HELP_MANIFEST = manifest
export function helpArticle(id: string): string {
  const topic = manifest.articles.find(a => a.id === id)
  return topic ? documents[`../../../src-misc/help/${topic.file}`] ?? '' : ''
}
export function rankHelp(query: string) {
  const terms = query.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(t => t.length > 2)
  return manifest.articles.map((a, index) => {
    const title = `${a.title} ${a.aliases.join(' ')}`.toLowerCase()
    const body = helpArticle(a.id).toLowerCase()
    return { ...a, score: terms.reduce((n, t) => n + (title.includes(t) ? 8 : body.includes(t) ? 1 : 0), 0), index }
  }).filter(a => !terms.length || a.score > 0).sort((a, b) => b.score - a.score || a.index - b.index)
}
// Read the real registries at bundle time; no separately maintained command list.
export function helpRegistry(name: 'COMMANDS' | 'KEYBINDINGS') {
  const source = commandSource.split(`pub const ${name}:`)[1]?.split('];')[0] ?? ''
  return [...source.matchAll(/\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,?\s*\)/g)].map(m => ({ key: m[1], description: m[2] }))
}
export function parseHelpAnswer(raw: string) {
  const v = JSON.parse(raw)
  if (!v || typeof v !== 'object' || Object.keys(v).some(key => !['answer', 'articles', 'navigation', 'guide'].includes(key)) || (v.navigation !== undefined && v.navigation !== null && typeof v.navigation !== 'string') || (v.guide !== undefined && v.guide !== null && typeof v.guide !== 'string') || typeof v.answer !== 'string' || !v.answer.trim() || !Array.isArray(v.articles) || v.articles.some((id: unknown) => !manifest.articles.some(a => a.id === id)) || (v.navigation && !manifest.navigation.includes(v.navigation)) || (v.guide && !manifest.workflows.includes(v.guide))) throw new Error('Help returned invalid guidance. Retry or use Reference and Guides.')
  return v as { answer: string; articles: string[]; navigation?: string | null; guide?: string | null }
}
