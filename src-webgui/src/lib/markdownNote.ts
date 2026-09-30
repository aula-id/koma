// A small markdown subset for the diagram detail editor.
// The editor shows the formatted text and stores this markdown.

export type NoteBlock =
  | { kind: 'p' | 'h1' | 'h2' | 'h3' | 'quote'; text: string }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'code'; text: string }
  | { kind: 'image'; alt: string; src: string }

export type NoteTextBlock = Extract<NoteBlock, { kind: 'p' | 'h1' | 'h2' | 'h3' | 'quote' }>

export function isNoteText(block: NoteBlock): block is NoteTextBlock {
  return block.kind === 'p' || block.kind === 'h1' || block.kind === 'h2' || block.kind === 'h3' || block.kind === 'quote'
}

const IMAGE_LINE = /^!\[([^\]]*)\]\(([^)\s]+)\)\s*$/
const HEADING = /^(#{1,3})\s+(.*)$/
const QUOTE = /^>\s?(.*)$/
const BULLET = /^[-*]\s+(.*)$/
const ORDERED = /^\d+\.\s+(.*)$/

export function parseNote(markdown: string): NoteBlock[] {
  const lines = markdown.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n')
  const blocks: NoteBlock[] = []
  let i = 0
  const pushMixed = (text: string) => {
    EMBEDDED_IMAGE.lastIndex = 0
    let last = 0
    let m: RegExpExecArray | null
    while ((m = EMBEDDED_IMAGE.exec(text))) {
      const src = imageSrcName(m[2] ?? '')
      const bang = m[0].startsWith('!')
      if (!noteImageFile(src) && !bang) continue
      const before = text.slice(last, m.index)
      if (before.trim()) blocks.push({ kind: 'p', text: before.trim() })
      blocks.push({ kind: 'image', alt: bang ? (m[1] ?? '') : (m[1] ?? ''), src })
      last = (m.index ?? 0) + m[0].length
    }
    const rest = text.slice(last)
    if (rest.trim()) blocks.push({ kind: 'p', text: rest.trim() })
    else if (last === 0 && text.trim()) blocks.push({ kind: 'p', text })
  }
  while (i < lines.length) {
    const line = lines[i] ?? ''
    if (line.trim() === '') {
      i++
      continue
    }
    const openFence = /^(```+)([^`]*)$/.exec(line)
    const opened = openFence?.[1]?.length ?? 0
    if (opened >= 3) {
      const body: string[] = []
      i++
      while (i < lines.length && fenceSize(lines[i] ?? '') < opened) {
        body.push(lines[i] ?? '')
        i++
      }
      if (i < lines.length) i++
      blocks.push({ kind: 'code', text: body.join('\n') })
      continue
    }
    const image = line.match(IMAGE_LINE)
    if (image) {
      blocks.push({ kind: 'image', alt: image[1] ?? '', src: image[2] ?? '' })
      i++
      continue
    }
    const heading = line.match(HEADING)
    if (heading) {
      const marks = heading[1] ?? '#'
      const kind = marks.length === 1 ? 'h1' : marks.length === 2 ? 'h2' : 'h3'
      blocks.push({ kind, text: heading[2] ?? '' })
      i++
      continue
    }
    if (QUOTE.test(line)) {
      const text: string[] = []
      while (i < lines.length && QUOTE.test(lines[i] ?? '')) {
        text.push((lines[i] ?? '').replace(QUOTE, '$1'))
        i++
      }
      blocks.push({ kind: 'quote', text: text.join('\n') })
      continue
    }
    if (BULLET.test(line) || ORDERED.test(line)) {
      const ordered = ORDERED.test(line)
      const items: string[] = []
      const pattern = ordered ? ORDERED : BULLET
      while (i < lines.length && pattern.test(lines[i] ?? '')) {
        items.push((lines[i] ?? '').replace(pattern, '$1'))
        i++
      }
      blocks.push({ kind: ordered ? 'ol' : 'ul', items })
      continue
    }
    const text: string[] = []
    while (i < lines.length && (lines[i] ?? '').trim() !== '' && !blockStart(lines[i] ?? '')) {
      text.push(lines[i] ?? '')
      i++
    }
    pushMixed(text.join('\n'))
  }
  return blocks
}

const EMBEDDED_IMAGE = /!?\[([^\]]*)\]\(([^)\s]+)\)/g

function imageSrcName(src: string): string {
  return src.replace(/\\/g, '/').split('/').pop() ?? src
}

function fenceSize(line: string): number {
  const trimmed = line.trim()
  return /^`+$/.test(trimmed) ? trimmed.length : 0
}

function blockStart(line: string): boolean {
  return /^(```+)/.test(line) || IMAGE_LINE.test(line) || HEADING.test(line) || QUOTE.test(line) || BULLET.test(line) || ORDERED.test(line)
}

const COMPOSER_IMAGE_MARKER = /^\s*\[Image #(\d+)\]\s*$/

/** Canonical markdown for diagram detail (images on their own lines). */
export function normalizeDiagramNoteMarkdown(markdown: string): string {
  const stripped = markdown
    .replace(/\[Image #\d+\]/g, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
  if (!stripped) return ''
  return serializeNote(parseNote(stripped))
}

/** Workspace image basenames already referenced in note markdown. */
export function referencedNoteImageNames(markdown: string): Set<string> {
  const names = new Set<string>()
  for (const block of parseNote(markdown)) {
    if (block.kind === 'image' && noteImageFile(block.src)) names.add(block.src)
  }
  for (const m of markdown.matchAll(/!\[[^\]]*]\(([^)\s]+)\)/g)) {
    const base = m[1]?.split(/[/\\]/).pop() ?? ''
    if (noteImageFile(base)) names.add(base)
  }
  return names
}

/** Replace mistaken composer `[Image #N]` markers with `![…](file)` when files exist. */
export function repairComposerImageMarkers(markdown: string, spareImageNames: string[]): string {
  if (!/\[Image #\d+\]/.test(markdown)) return markdown
  const spare = [...spareImageNames]
  let index = 0
  const lines = markdown.replace(/\r\n/g, '\n').split('\n')
  const out: string[] = []
  for (const line of lines) {
    if (COMPOSER_IMAGE_MARKER.test(line)) {
      const name = spare[index++]
      if (name) out.push(`![image](${name})`)
      continue
    }
    out.push(line.replace(/\[Image #\d+\]/g, () => {
      const name = spare[index++]
      return name ? `![image](${name})` : ''
    }))
  }
  return out.join('\n')
}

export function serializeNote(blocks: NoteBlock[]): string {
  const parts: string[] = []
  for (const block of blocks) {
    if (block.kind === 'code') {
      const mark = block.text.includes('```') ? '````' : '```'
      parts.push(`${mark}\n${block.text}\n${mark}`)
      continue
    }
    if (block.kind === 'image') {
      if (!block.src.trim()) continue
      parts.push(`![${block.alt.replace(/[\[\]]/g, '')}](${block.src.trim()})`)
      continue
    }
    if (block.kind === 'ul' || block.kind === 'ol') {
      if (!block.items.length) continue
      parts.push(block.items.map((item, index) => (block.kind === 'ol' ? `${index + 1}. ${item}` : `- ${item}`)).join('\n'))
      continue
    }
    if (!isNoteText(block)) continue
    if (!block.text.trim() && block.kind === 'p') continue
    if (block.kind === 'h1') parts.push(`# ${hardBreakLines(block.text)}`)
    else if (block.kind === 'h2') parts.push(`## ${hardBreakLines(block.text)}`)
    else if (block.kind === 'h3') parts.push(`### ${hardBreakLines(block.text)}`)
    else if (block.kind === 'quote') parts.push(hardBreakLines(block.text).split('\n').map((line) => `> ${line}`).join('\n'))
    else parts.push(hardBreakLines(block.text))
  }
  return parts.join('\n\n')
}

/** Keep Shift+Enter as a markdown hard break (`\` + newline), not a new paragraph. */
function hardBreakLines(text: string): string {
  const lines = text.split('\n')
  if (lines.length <= 1) return text
  return lines.map((line, index) => (index < lines.length - 1 ? `${line.replace(/\\$/, '')}\\` : line)).join('\n')
}

export function safeNoteUrl(url: string): boolean {
  if (/^(https?:\/\/|mailto:)/i.test(url)) return !/[\s<>"']/.test(url)
  if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return false
  if (url.startsWith('//') || url.startsWith('/') || url.includes('..') || url.includes('\\') || url.includes('/')) return false
  return /^[^/\s]+$/.test(url)
}

export function noteImageFile(src: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._-]*\.(png|jpe?g|gif|webp)$/i.test(src)
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function escapeAttr(text: string): string {
  return escapeHtml(text).replace(/"/g, '&quot;')
}

/** Inline markdown (`**bold**`, `*italic*`, `` `code` ``, links) as HTML. */
export function inlineToHtml(source: string): string {
  return formatInline(source)
}

function formatInline(source: string): string {
  let i = 0
  let html = ''
  while (i < source.length) {
    if (source.startsWith('**', i)) {
      const close = source.indexOf('**', i + 2)
      if (close !== -1 && !source.slice(i + 2, close).includes('\n')) {
        html += `<strong>${formatInline(source.slice(i + 2, close))}</strong>`
        i = close + 2
        continue
      }
    }
    if (source[i] === '*' && source[i + 1] !== '*') {
      const close = source.indexOf('*', i + 1)
      if (close !== -1 && !source.slice(i + 1, close).includes('\n')) {
        html += `<em>${formatInline(source.slice(i + 1, close))}</em>`
        i = close + 1
        continue
      }
    }
    if (source[i] === '`') {
      const close = source.indexOf('`', i + 1)
      if (close !== -1 && !source.slice(i + 1, close).includes('\n')) {
        html += `<code>${escapeHtml(source.slice(i + 1, close))}</code>`
        i = close + 1
        continue
      }
    }
    if (source[i] === '[') {
      const match = /^\[([^\]]+)\]\(([^)\s]+)\)/.exec(source.slice(i))
      if (match && safeNoteUrl(match[2] ?? '')) {
        html += `<a href="${escapeAttr(match[2] ?? '')}">${formatInline(match[1] ?? '')}</a>`
        i += match[0].length
        continue
      }
    }
    if (source[i] === '\n') {
      html += '<br>'
      i++
      continue
    }
    const rest = source.slice(i + 1)
    const next = rest.search(/[*`[\n]/)
    const end = next === -1 ? source.length : i + 1 + next
    html += escapeHtml(source.slice(i, end))
    i = end
  }
  return html
}

function decodeEntities(text: string): string {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
}

/** Inverse of inlineToHtml for the tags the editor emits, plus `<b>` and `<i>`. */
export function inlineFromHtml(html: string): string {
  const trimmed = html.trim()
  if (!trimmed || trimmed === '<br>' || trimmed === '<br/>' || trimmed === '<br />') return ''
  let i = 0
  let out = ''
  const stack: { tag: string; href: string }[] = []
  while (i < html.length) {
    if (html[i] !== '<') {
      const next = html.indexOf('<', i)
      const end = next === -1 ? html.length : next
      out += decodeEntities(html.slice(i, end))
      i = end
      continue
    }
    const close = html.indexOf('>', i)
    if (close === -1) {
      out += decodeEntities(html.slice(i))
      break
    }
    const raw = html.slice(i + 1, close).trim()
    i = close + 1
    const closing = raw.startsWith('/')
    const name = (closing ? raw.slice(1) : raw).split(/\s+/)[0]?.toLowerCase().replace(/\/$/, '') ?? ''
    const self = raw.endsWith('/') || name === 'br'
    if (name === 'br') {
      out += '\n'
      continue
    }
    if (!name || name === 'span' || name === 'div' || name === 'p' || name === 'font') continue
    if (closing) {
      const open = stack.pop()
      if (!open) continue
      if (open.tag === 'strong') out += '**'
      else if (open.tag === 'em') out += '*'
      else if (open.tag === 'code') out += '`'
      else if (open.tag === 'a') out += `](${open.href})`
      continue
    }
    if (self) continue
    const tag = name === 'b' ? 'strong' : name === 'i' ? 'em' : name
    if (tag !== 'strong' && tag !== 'em' && tag !== 'code' && tag !== 'a') continue
    const hrefMatch = /\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i.exec(raw)
    const href = hrefMatch?.[1] ?? hrefMatch?.[2] ?? hrefMatch?.[3] ?? ''
    stack.push({ tag, href })
    if (tag === 'strong') out += '**'
    else if (tag === 'em') out += '*'
    else if (tag === 'code') out += '`'
    else out += '['
  }
  return out.replace(/\n$/, '')
}
