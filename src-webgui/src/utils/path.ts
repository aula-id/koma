// Cross-platform path display helpers for the webgui.
//
// The daemon may push absolute Windows paths (including the `\\?\` extended-length
// prefix). Naive `split('/')` basename helpers leave the full path as the "name",
// which blows up Explore "File changed" rows and forces horizontal scroll.

/** Strip Windows extended-length / device prefixes so the rest looks like a normal path. */
export function stripWinPrefix(path: string): string {
  if (!path) return path
  // `\\?\C:\...` or `//?/C:/...`
  if (path.startsWith('\\\\?\\') || path.startsWith('//?/')) return path.slice(4)
  // `\\.\C:\...` device namespace
  if (path.startsWith('\\\\.\\') || path.startsWith('//./')) return path.slice(4)
  return path
}

/** Last path segment, accepting `/` and `\` separators (and Windows prefixes). */
export function baseName(path: string): string {
  if (!path) return path
  const cleaned = stripWinPrefix(path).replace(/\\/g, '/')
  const parts = cleaned.split('/').filter(Boolean)
  return parts[parts.length - 1] || path
}

/** Parent directory using the same separator-tolerant logic as {@link baseName}. */
export function dirName(path: string): string {
  if (!path) return ''
  const cleaned = stripWinPrefix(path).replace(/\\/g, '/')
  const parts = cleaned.split('/').filter(Boolean)
  if (parts.length <= 1) return ''
  parts.pop()
  // Preserve a drive root like `C:` when present.
  if (/^[A-Za-z]:$/.test(parts[0] ?? '')) {
    const drive = parts.shift()!
    return parts.length === 0 ? `${drive}/` : `${drive}/${parts.join('/')}`
  }
  return parts.join('/')
}

/** Parent path for coding-panel relative paths (usually POSIX-ish). */
export function parentDirPath(path: string): string {
  if (!path) return ''
  const cleaned = path.replace(/\\/g, '/')
  const idx = cleaned.lastIndexOf('/')
  return idx <= 0 ? '' : cleaned.slice(0, idx)
}
