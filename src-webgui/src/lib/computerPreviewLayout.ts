export type PreviewBounds = { x: number; y: number; width: number; height: number }

export const COMPUTER_PREVIEW_MIN_WIDTH = 250
export const COMPUTER_PREVIEW_MIN_HEIGHT = 180
const DEFAULT_WIDTH = 560
const DEFAULT_HEIGHT = 315

/** Keep the box the user sized. Width and height stay independent.
 *  A new frame does not change them. A missing or collapsed size (below
 *  250×180, including a saved 1×1) uses the 560×315 default. The viewport
 *  can shrink a side that would leave the screen, but never below that floor.
 *  A box that misses the viewport returns to the default corner. */
export function fitComputerPreview(value: Partial<PreviewBounds>, viewport: { width: number; height: number }): PreviewBounds {
  const givenWidth = Number.isFinite(value.width) ? value.width! : NaN
  const givenHeight = Number.isFinite(value.height) ? value.height! : NaN
  const wantWidth = givenWidth >= COMPUTER_PREVIEW_MIN_WIDTH ? givenWidth : DEFAULT_WIDTH
  const wantHeight = givenHeight >= COMPUTER_PREVIEW_MIN_HEIGHT ? givenHeight : DEFAULT_HEIGHT
  const maxWidth = Math.max(COMPUTER_PREVIEW_MIN_WIDTH, viewport.width - 24)
  const maxHeight = Math.max(COMPUTER_PREVIEW_MIN_HEIGHT, viewport.height - 70)
  const width = Math.min(wantWidth, maxWidth)
  const height = Math.min(wantHeight, maxHeight)
  const defaultX = Math.max(12, viewport.width - width - 24)
  const defaultY = 70
  let x = Number.isFinite(value.x) ? value.x! : defaultX
  let y = Number.isFinite(value.y) ? value.y! : defaultY
  const hits = viewport.width > 0 && viewport.height > 0
    && x < viewport.width && y < viewport.height
    && x + width > 0 && y + height > 0
  if (!hits) {
    x = defaultX
    y = defaultY
  }
  return {
    width, height,
    x: Math.max(12, Math.min(x, Math.max(12, viewport.width - width - 12))),
    y: Math.max(40, Math.min(y, Math.max(40, viewport.height - height - 12))),
  }
}
