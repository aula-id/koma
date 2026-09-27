import { extOf } from './viewerKind'

export function isMarkdownPath(path: string): boolean {
  const ext = extOf(path)
  return ext === 'md' || ext === 'markdown'
}

export function codingTabId(root: string, path: string, preview = false): string {
  return `${preview ? 'coding-preview' : 'coding'}:${root}:${path}`
}

/** Resolve a document-relative URL within its workspace, never outside it. */
export function markdownLocalPath(documentPath: string, url: string): string | null {
  if (!url || url.startsWith('#') || url.startsWith('//') || /^[a-z][a-z\d+.-]*:/i.test(url)) return null
  let path: string
  try {
    path = decodeURIComponent(url.split(/[?#]/, 1)[0])
  } catch {
    return null
  }
  if (!path || /[\x00-\x1f\\]/.test(path)) return null
  const parts = path.startsWith('/') ? [] : documentPath.split('/').slice(0, -1)
  for (const part of path.split('/')) {
    if (!part || part === '.') continue
    if (part === '..') {
      if (!parts.length) return null
      parts.pop()
    } else {
      parts.push(part)
    }
  }
  return parts.length ? parts.join('/') : null
}
