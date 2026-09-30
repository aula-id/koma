import { viewerKindForPath } from './viewerKind'

/** Parse `@path` / `@[N]path` wire token into workspace root + relative path. */
export function parseFileRefWire(
  wire: string,
  workdirs: readonly string[],
): { root: string; path: string } | null {
  const w = wire.trim()
  if (!w.startsWith('@')) return null
  let body = w.slice(1)
  const range = body.match(/:(\d+)(?:-(\d+))?$/)
  if (range) body = body.slice(0, body.length - range[0].length)
  const multi = body.match(/^\[(\d+)\]([\s\S]*)$/)
  if (multi) {
    const idx = Number(multi[1])
    const root = workdirs[idx]
    if (!root) return null
    const path = (multi[2] || '.').replace(/\\/g, '/')
    return { root, path }
  }
  const root = workdirs[0]
  if (!root) return null
  return { root, path: body.replace(/\\/g, '/').replace(/^\/+/, '') || '.' }
}

export function localFileTabId(absPath: string): string {
  return `local:${absPath}`
}

export function codingTabTitleFromPath(path: string): string {
  const base = path.replace(/\\/g, '/').split('/').pop()
  return base || path
}

export function shouldOpenAsCodingTab(absPath: string): boolean {
  const kind = viewerKindForPath(absPath)
  return kind === 'text' || kind === 'pdf' || kind === 'sqlite' || kind === 'docx' || kind === 'excel'
}
