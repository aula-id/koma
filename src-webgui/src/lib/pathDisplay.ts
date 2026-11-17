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

/** Windows-shaped path: drive letter, `\\` (UNC / verbatim) or `//?/` prefix. Decided from the path itself, never the host. */
export function isWindowsPath(p: string): boolean {
  return /^[A-Za-z]:([\\/]|$)/.test(p) || p.startsWith('\\\\') || p.startsWith('//?/')
}

/** Upper-case a leading Windows drive letter ("c:/x" -> "C:/x"); no-op for anything else. */
export function foldDrive(p: string): string {
  return /^[A-Za-z]:/.test(p) ? p[0].toUpperCase() + p.slice(1) : p
}

/** Strip the verbatim prefix and turn `\\` into `/`, only for Windows-shaped paths. POSIX paths are untouched. */
export function toSlashIfWindows(p: string): string {
  if (!isWindowsPath(p)) return p
  return stripVerbatim(p).replace(/\\/g, '/')
}

const trimSlash = (p: string): string => (p.length > 1 ? p.replace(/\/+$/, '') || '/' : p)

function winKey(p: string): string {
  return trimSlash(foldDrive(toSlashIfWindows(p))).toLowerCase()
}

/** Comparison key: Windows-shaped paths are slash-normalised and case-folded; POSIX-shaped only lose a trailing `/`. */
export function pathKey(p: string): string {
  return isWindowsPath(p) ? winKey(p) : trimSlash(p)
}

export function pathsEqual(a: string, b: string): boolean {
  return pathKey(a) === pathKey(b)
}

/** '' when equal, the relative tail (original case) when `abs` is under `root`, else null. */
export function relativeUnder(root: string, abs: string): string | null {
  const win = isWindowsPath(root) || isWindowsPath(abs)
  const norm = win ? foldDrive(toSlashIfWindows(abs)) : abs
  const r = win ? winKey(root) : trimSlash(root)
  const k = win ? winKey(abs) : trimSlash(abs)
  if (k === r) return ''
  const prefix = r.endsWith('/') ? r : `${r}/`
  if (!k.startsWith(prefix)) return null
  return norm.slice(prefix.length)
}

export const rootLabel = (root: string): string => pathBasename(root)
