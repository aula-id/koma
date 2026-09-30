// Heuristics for pasting markdown / URLs into the composer Lexical field.

/** True when plain-text paste should be parsed as markdown blocks/inlines. */
export function looksLikeComposerMarkdown(text: string): boolean {
  const sample = text.replace(/\r\n/g, '\n')
  if (sample.includes('```')) return true
  if (/^\s{0,3}#{1,6}\s/m.test(sample)) return true
  if (/^\s*[-*+]\s*\[[ xX]\]\s/m.test(sample)) return true
  if (/^\s*[-*+]\s/m.test(sample)) return true
  if (/^\s*\d+\.\s/m.test(sample)) return true
  if (/^\s*>\s/m.test(sample)) return true
  if (/\*\*.+\*\*|__.+__|\[[^\]]+\]\([^)]+\)|`[^`]+`/.test(sample)) return true
  return false
}

const LONE_URL = /^https?:\/\/\S+$/i

export function loneHttpUrl(text: string): string | null {
  const trimmed = text.trim()
  return LONE_URL.test(trimmed) ? trimmed : null
}
