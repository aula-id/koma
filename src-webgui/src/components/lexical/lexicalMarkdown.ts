import { $isCodeNode, type CodeNode } from '@lexical/code'
import { $isLinkNode, type LinkNode } from '@lexical/link'
import { $isListItemNode, $isListNode, type ListItemNode, type ListNode } from '@lexical/list'
import { $isHeadingNode, $isQuoteNode, type HeadingNode, type QuoteNode } from '@lexical/rich-text'
import {
  $getRoot,
  $isElementNode,
  $isLineBreakNode,
  $isTextNode,
  type LexicalNode,
  type TextNode,
} from 'lexical'
import { $isComposerChipNode } from './chipNodes'
import { $isNoteImageNode } from './noteImageNode'

type Marks = { bold: boolean; italic: boolean; strike: boolean; code: boolean }

const NONE: Marks = { bold: false, italic: false, strike: false, code: false }

function marksOf(node: TextNode): Marks {
  return {
    bold: node.hasFormat('bold'),
    italic: node.hasFormat('italic'),
    strike: node.hasFormat('strikethrough'),
    code: node.hasFormat('code'),
  }
}

function sameMarks(a: Marks, b: Marks): boolean {
  return a.bold === b.bold && a.italic === b.italic && a.strike === b.strike && a.code === b.code
}

/** Keep literal characters from being read as markdown once the style markers are added. */
export function escapeMarkdownText(text: string): string {
  return text.replace(/[\\`*_~\[]/g, '\\$&')
}

export function codeSpan(text: string): string {
  const runs = text.match(/`+/g)
  const longest = runs ? Math.max(...runs.map((run) => run.length)) : 0
  const fence = '`'.repeat(Math.max(1, longest + 1))
  const pad = text.startsWith('`') || text.endsWith('`') ? ' ' : ''
  return `${fence}${pad}${text}${pad}${fence}`
}

export function wrapMarks(text: string, marks: Marks): string {
  if (!text) return ''
  let body = marks.code ? codeSpan(text) : escapeMarkdownText(text)
  if (marks.strike) body = `~~${body}~~`
  if (marks.bold && marks.italic) body = `***${body}***`
  else if (marks.bold) body = `**${body}**`
  else if (marks.italic) body = `*${body}*`
  return body
}

function fenceFor(body: string): string {
  const runs = body.match(/`{3,}/g)
  const longest = runs ? Math.max(...runs.map((run) => run.length)) : 2
  return '`'.repeat(Math.max(3, longest + 1))
}

function exportInlines(nodes: LexicalNode[]): string {
  let out = ''
  let buf = ''
  let marks = NONE
  const flush = () => {
    out += wrapMarks(buf, marks)
    buf = ''
  }
  for (const node of nodes) {
    if ($isLineBreakNode(node)) {
      flush()
      out += '\\\n'
      marks = NONE
      continue
    }
    if ($isTextNode(node)) {
      const next = marksOf(node)
      if (buf && !sameMarks(marks, next)) flush()
      marks = next
      buf += node.getTextContent()
      continue
    }
    flush()
    marks = NONE
    if ($isComposerChipNode(node) || $isNoteImageNode(node)) {
      out += node.getTextContent()
      continue
    }
    if ($isLinkNode(node)) {
      out += exportLink(node)
      continue
    }
    if ($isElementNode(node)) out += exportInlines(node.getChildren())
  }
  flush()
  return out
}

function exportLink(node: LinkNode): string {
  const label = exportInlines(node.getChildren())
  const url = node.getURL().replace(/[()]/g, (char) => `\\${char}`)
  return `[${label}](${url})`
}

function exportList(list: ListNode, depth: number): string {
  const kind = list.getListType()
  let index = list.getStart()
  return list.getChildren().map((item) => {
    if (!$isListItemNode(item)) return exportBlock(item, depth)
    const indent = '  '.repeat(depth)
    const marker = kind === 'number' ? `${index++}. ` : kind === 'check' ? `- [${item.getChecked() ? 'x' : ' '}] ` : '- '
    return exportListItem(item, indent, marker, depth)
  }).join('\n')
}

function exportListItem(item: ListItemNode, indent: string, marker: string, depth: number): string {
  const lines: string[] = []
  let first = true
  for (const child of item.getChildren()) {
    if ($isListNode(child)) {
      lines.push(exportList(child, depth + 1))
      first = false
      continue
    }
    const block = exportBlock(child, depth)
    if (first) {
      lines.push(`${indent}${marker}${block}`)
      first = false
      continue
    }
    const pad = `${indent}${' '.repeat(marker.length)}`
    lines.push(block.split('\n').map((line) => `${pad}${line}`).join('\n'))
  }
  return lines.join('\n')
}

function exportQuote(node: QuoteNode): string {
  const inner = node.getChildren().map((child) => exportBlock(child, 0)).join('\n\n')
  return inner.split('\n').map((line) => `> ${line}`).join('\n')
}

function exportCode(node: CodeNode): string {
  const body = node.getTextContent().replace(/\n$/, '')
  const fence = fenceFor(body)
  const lang = node.getLanguage() ?? ''
  return `${fence}${lang}\n${body}\n${fence}`
}

function exportHeading(node: HeadingNode): string {
  const level = Number(node.getTag().replace('h', '')) || 1
  return `${'#'.repeat(Math.min(6, Math.max(1, level)))} ${exportInlines(node.getChildren())}`
}

function exportBlock(node: LexicalNode, depth: number): string {
  if ($isCodeNode(node)) return exportCode(node)
  if ($isHeadingNode(node)) return exportHeading(node)
  if ($isQuoteNode(node)) return exportQuote(node)
  if ($isListNode(node)) return exportList(node, depth)
  if ($isElementNode(node)) return exportInlines(node.getChildren())
  return node.getTextContent()
}

/** CommonMark for the current editor state. Line breaks stay line breaks. */
export function $exportLexicalMarkdown(): string {
  const blocks = $getRoot().getChildren().map((node) => exportBlock(node, 0))
  if (blocks.every((block) => block === '')) return ''
  return blocks.join('\n\n')
}
