import { shouldCollapsePaste } from './pasteText.ts'

// Heuristics for pasting markdown / URLs into the composer Lexical field.

const TABLE_ROW = /^\s*\|?.+\|.+\|?\s*$/
const TABLE_DIVIDER = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/

/** GFM task marker left in a list item after `- ` was already consumed. */
export function splitTaskListMarker(text: string): { checked: boolean; rest: string } | null {
  const match = text.match(/^\[(\s|x)\]\s+/i)
  if (!match) return null
  return { checked: /^x$/i.test(match[1] ?? ''), rest: text.slice(match[0].length) }
}

/** How the composer should handle pasted plain text. Long / multi-line bodies
 *  collapse to a `[Pasted Text #N]` chip (TUI parity) even when they happen to
 *  contain markdown-looking markers. Short markdown stays formatted in-place. */
export function composerTextPasteAction(text: string): 'collapse' | 'markdown' | 'inline' {
  const normalized = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
  if (!normalized) return 'inline'
  if (shouldCollapsePaste(normalized)) return 'collapse'
  if (looksLikeComposerMarkdown(normalized)) return 'markdown'
  return 'inline'
}

/** True when plain-text paste should be parsed as markdown blocks/inlines. */
export function looksLikeComposerMarkdown(text: string): boolean {
  const sample = text.replace(/\r\n/g, '\n')
  if (sample.includes('```')) return true
  if (/^\s{0,3}#{1,6}\s/m.test(sample)) return true
  if (/^\s*[-*+]\s*\[[ xX]\]\s/m.test(sample)) return true
  if (/^\s*[-*+]\s/m.test(sample)) return true
  if (/^\s*\d+\.\s/m.test(sample)) return true
  if (/^\s*>\s/m.test(sample)) return true
  if (TABLE_ROW.test(sample) || TABLE_DIVIDER.test(sample)) return true
  if (/^\s*\|.+\|\s*$/m.test(sample)) return true
  if (/\*\*.+\*\*|__.+__|\[[^\]]+\]\([^)]+\)|`[^`]+`/.test(sample)) return true
  return false
}

/** Prefer clipboard HTML when GitHub/browser copies rendered rich text. */
export function markdownFromClipboardHtml(html: string): string | null {
  const trimmed = html.trim()
  if (!trimmed || trimmed === '<br>' || trimmed === '<br/>') return null
  if (!/<(?:a|strong|b|em|i|code|ul|ol|li|h[1-6]|blockquote|pre|table|p|div)\b/i.test(trimmed)) {
    return null
  }
  return htmlToComposerMarkdown(trimmed)
}

function decodeEntities(text: string): string {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
}

/** Small HTML → markdown for paste (links, emphasis, lists, breaks). */
function htmlToComposerMarkdown(html: string): string {
  if (typeof DOMParser === 'undefined') return ''
  const doc = new DOMParser().parseFromString(html, 'text/html')
  return blockMarkdown(doc.body).trim()
}

function inlineMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return decodeEntities(node.textContent ?? '')
  if (node.nodeType !== Node.ELEMENT_NODE) return ''
  const el = node as HTMLElement
  const tag = el.tagName.toLowerCase()
  const inner = () => Array.from(el.childNodes).map(inlineMarkdown).join('')
  if (tag === 'br') return '\n'
  if (tag === 'strong' || tag === 'b') return `**${inner()}**`
  if (tag === 'em' || tag === 'i') return `*${inner()}*`
  if (tag === 'code') return `\`${inner()}\``
  if (tag === 'a') {
    const href = el.getAttribute('href')?.trim() ?? ''
    const label = inner() || href
    return href ? `[${label}](${href})` : label
  }
  return inner()
}

function listItemMarkdown(el: HTMLElement): string {
  const box = el.querySelector(':scope > input[type="checkbox"]')
  const checked =
    box instanceof HTMLInputElement
      ? box.checked
      : el.classList.contains('koma-checklist-checked') || el.getAttribute('aria-checked') === 'true'
  const isTask =
    !!box ||
    el.classList.contains('koma-checklist-item') ||
    el.classList.contains('task-list-item') ||
    el.getAttribute('role') === 'checkbox'
  const text = inlineMarkdown(el).trim()
  if (isTask) return `- [${checked ? 'x' : ' '}] ${text}`
  return `- ${text}`
}

function blockMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) {
    const t = decodeEntities(node.textContent ?? '').trim()
    return t ? `${t}\n\n` : ''
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return ''
  const el = node as HTMLElement
  const tag = el.tagName.toLowerCase()
  if (tag === 'br') return '\n'
  if (tag === 'ul') {
    return `${Array.from(el.children)
      .filter((child) => child.tagName.toLowerCase() === 'li')
      .map((li) => listItemMarkdown(li as HTMLElement))
      .join('\n')}\n\n`
  }
  if (tag === 'ol') {
    let n = 1
    return `${Array.from(el.children)
      .filter((child) => child.tagName.toLowerCase() === 'li')
      .map((li) => `${n++}. ${inlineMarkdown(li).trim()}`)
      .join('\n')}\n\n`
  }
  if (tag === 'li') return `${listItemMarkdown(el)}\n`
  if (/^h[1-6]$/.test(tag)) {
    const level = Number(tag[1]) || 1
    return `${'#'.repeat(Math.min(6, level))} ${inlineMarkdown(el).trim()}\n\n`
  }
  if (tag === 'blockquote') {
    return `${inlineMarkdown(el)
      .trim()
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n')}\n\n`
  }
  if (tag === 'pre') {
    const code = el.textContent ?? ''
    return `\`\`\`\n${code.replace(/\n$/, '')}\n\`\`\`\n\n`
  }
  if (tag === 'p' || tag === 'div') {
    const text = inlineMarkdown(el).trim()
    return text ? `${text}\n\n` : Array.from(el.childNodes).map(blockMarkdown).join('')
  }
  return Array.from(el.childNodes).map(blockMarkdown).join('')
}

const LONE_URL = /^https?:\/\/\S+$/i

export function loneHttpUrl(text: string): string | null {
  const trimmed = text.trim()
  return LONE_URL.test(trimmed) ? trimmed : null
}
