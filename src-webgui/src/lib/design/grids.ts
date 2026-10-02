import type { DesignLayoutGrid } from './types'

export type DesignGridBand = { x: number; y: number; w: number; h: number }

/** Column and row bands for a board grid. Square grids return no bands. */
export function layoutGridBands(grid: DesignLayoutGrid, boxW: number, boxH: number): DesignGridBand[] {
  if (grid.kind === 'square') return []
  const span = grid.kind === 'column' ? boxW : boxH
  const gutter = grid.gutter ?? 0
  const align = grid.align ?? 'stretch'
  const size = grid.size ?? 8
  let count = grid.count ?? 0
  if (align === 'stretch') count = Math.max(1, count || 3)
  else if (count <= 0) {
    const usable = Math.max(0, span - (grid.offset ?? 0))
    count = size > 0 ? Math.max(1, Math.floor((usable + gutter) / (size + gutter))) : 1
  }
  count = Math.max(1, Math.round(count))
  let cell = size
  let origin = grid.offset ?? 0
  if (align === 'stretch') {
    cell = Math.max(1, (span - origin * 2 - gutter * Math.max(0, count - 1)) / count)
  } else {
    cell = Math.max(1, size)
    const run = count * cell + gutter * Math.max(0, count - 1)
    if (align === 'end') origin = span - (grid.offset ?? 0) - run
    else if (align === 'center') origin = (span - run) / 2
  }
  return Array.from({ length: count }, (_, at) => {
    const start = origin + at * (cell + gutter)
    return grid.kind === 'column'
      ? { x: start, y: 0, w: cell, h: boxH }
      : { x: 0, y: start, w: boxW, h: cell }
  })
}
