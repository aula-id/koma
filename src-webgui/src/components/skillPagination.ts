/** Height of one skill row (`min-h-[42px]` in SkillsPanel). */
export const SKILL_ROW_PX = 42
/** Page size used until the list has been measured. */
export const SKILL_PAGE_FALLBACK = 50
const LIST_PADDING_PX = 8
const MIN_PAGE_SIZE = 3

/** How many rows fit in a list of `listHeightPx` without scrolling. */
export function skillPageSize(listHeightPx: number): number {
  if (!(listHeightPx > 0)) return SKILL_PAGE_FALLBACK
  return Math.max(MIN_PAGE_SIZE, Math.floor((listHeightPx - LIST_PADDING_PX) / SKILL_ROW_PX))
}

export function pageCount(total: number, size: number): number {
  return Math.max(1, Math.ceil(total / size))
}

export function clampPage(page: number, total: number, size: number): number {
  return Math.min(Math.max(0, page), pageCount(total, size) - 1)
}

export function pageOfIndex(index: number, size: number): number {
  return index < 0 ? 0 : Math.floor(index / size)
}

/** Slice bounds of a page; `end` is exclusive. */
export function pageBounds(page: number, total: number, size: number): { start: number; end: number } {
  const start = Math.min(page * size, total)
  return { start, end: Math.min(start + size, total) }
}
