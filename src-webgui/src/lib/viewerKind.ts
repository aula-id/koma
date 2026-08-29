/** Extension → coding tab viewer kind. */

export type ViewerKind =
  | 'text'
  | 'image'
  | 'pdf'
  | 'video'
  | 'sqlite'
  | 'docx'
  | 'excel'

const IMAGE = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'bmp',
  'svg',
  'ico',
  'avif',
  'tif',
  'tiff',
])
const VIDEO = new Set(['mp4', 'webm', 'ogg', 'ogv', 'mov', 'mkv', 'm4v'])
const SQLITE = new Set(['db', 'sqlite', 'sqlite3'])
const EXCEL = new Set(['xlsx', 'xlsm', 'xls'])

export function extOf(path: string): string {
  const base = path.replace(/\\/g, '/').split('/').pop() ?? path
  const dot = base.lastIndexOf('.')
  if (dot <= 0) return ''
  return base.slice(dot + 1).toLowerCase()
}

export function viewerKindForPath(path: string): ViewerKind {
  const ext = extOf(path)
  if (!ext) return 'text'
  if (IMAGE.has(ext)) return 'image'
  if (ext === 'pdf') return 'pdf'
  if (VIDEO.has(ext)) return 'video'
  if (SQLITE.has(ext)) return 'sqlite'
  if (ext === 'docx') return 'docx'
  if (EXCEL.has(ext)) return 'excel'
  return 'text'
}

export function mimeForPath(path: string): string {
  const ext = extOf(path)
  switch (ext) {
    case 'png':
      return 'image/png'
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg'
    case 'gif':
      return 'image/gif'
    case 'webp':
      return 'image/webp'
    case 'bmp':
      return 'image/bmp'
    case 'svg':
      return 'image/svg+xml'
    case 'ico':
      return 'image/x-icon'
    case 'avif':
      return 'image/avif'
    case 'pdf':
      return 'application/pdf'
    case 'mp4':
    case 'm4v':
      return 'video/mp4'
    case 'webm':
      return 'video/webm'
    case 'ogg':
    case 'ogv':
      return 'video/ogg'
    case 'mov':
      return 'video/quicktime'
    case 'mkv':
      return 'video/x-matroska'
    case 'docx':
      return 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
    case 'xlsx':
    case 'xlsm':
      return 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
    default:
      return 'application/octet-stream'
  }
}
