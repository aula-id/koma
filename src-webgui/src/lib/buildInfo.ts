export interface BuildInfo {
  version: string
  channel: 'stable' | 'preview'
  sha: string
  target: string
  os: string
  buildDate: string | null
}

export const REPO_URL = 'https://github.com/aula-id/koma'
export const LICENSE_URL = 'https://github.com/aula-id/koma/blob/main/LICENSE'

const str = (v: unknown): string => (typeof v === 'string' && v ? v : 'unknown')

/** Tolerant reader for `window.__komaBuild`: anything malformed degrades to "unknown". */
export function readBuildInfo(raw: unknown): BuildInfo {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  return {
    version: str(o.version),
    channel: o.channel === 'preview' ? 'preview' : 'stable',
    sha: str(o.sha),
    target: str(o.target),
    os: str(o.os),
    buildDate: typeof o.buildDate === 'string' && o.buildDate ? o.buildDate : null,
  }
}

export function formatBuildInfo(i: BuildInfo): string {
  return [
    `Koma ${i.version} (${i.channel})`,
    `Commit: ${i.sha}`,
    `Target: ${i.target}`,
    `OS: ${i.os}`,
    `Built: ${i.buildDate ?? 'unknown'}`,
  ].join('\n')
}
