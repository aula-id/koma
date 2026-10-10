import { hasCodingPathDrag } from './codingRef'

const TAB_DRAG_MIME = 'application/x-koma-tab'

/** Finder, Explorer, and Linux file managers. Not an in-app tab or coding-tree drag. */
export function isExternalFileDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt?.types) return false
  const types = Array.from(dt.types)
  if (types.includes(TAB_DRAG_MIME) || hasCodingPathDrag(dt)) return false
  return (
    types.includes('Files') ||
    types.includes('text/uri-list') ||
    types.includes('application/x-moz-file')
  )
}
