/** Drop a Windows verbatim prefix: `\\?\C:\x` -> `C:\x`, `\\?\UNC\h\s` -> `\\h\s`. Other paths pass through. */
export function stripVerbatim(p: string): string {
  if (/^[\\/]{2}\?[\\/]UNC[\\/]/i.test(p)) return p.slice(0, 2) + p.slice(8)
  if (/^[\\/]{2}\?[\\/][A-Za-z]:(?:[\\/]|$)/.test(p)) return p.slice(4)
  return p
}

/** Last path component in either separator style; the whole input when there is none. */
export function pathBasename(p: string): string {
  const parts = stripVerbatim(p).split(/[/\\]/).filter(Boolean)
  return parts[parts.length - 1] || p
}

const encodeSegments = (path: string): string =>
  path.split('/').map((seg) => (/^[A-Za-z]:$/.test(seg) ? seg : encodeURIComponent(seg))).join('/')

/** `file://` URL for an absolute POSIX, drive-letter or UNC path in any separator style. */
export function fileUrl(abs: string): string {
  const norm = stripVerbatim(abs).replace(/\\/g, '/')
  if (norm.startsWith('//')) return `file:${encodeSegments(norm)}`
  if (norm.startsWith('/')) return `file://${encodeSegments(norm)}`
  return `file:///${encodeSegments(norm)}`
}
