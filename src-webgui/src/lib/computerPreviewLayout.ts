export type PreviewBounds = { x: number; y: number; width: number; height: number }

/** Fit a shared image; before selection, width and height are independent.
 *  Dragging uses this and may grow the panel. A source switch uses contain. */
export function fitComputerPreview(value: Partial<PreviewBounds>, aspect: number | null, viewport: { width: number; height: number }): PreviewBounds {
  const ratio = aspect !== null && Number.isFinite(aspect) && aspect > 0 ? aspect : null
  const maxWidth = Math.max(1, viewport.width - 24)
  const maxHeight = Math.max(1, viewport.height - 70)
  const limit = ratio ? Math.min(maxWidth, maxHeight * ratio) : maxWidth
  const minimum = Math.min(limit, ratio ? Math.max(240, 120 * ratio) : 240)
  const width = Math.max(minimum, Math.min(Number.isFinite(value.width) ? value.width! : 560, limit))
  const height = ratio ? width / ratio : Math.max(Math.min(120, maxHeight), Math.min(Number.isFinite(value.height) ? value.height! : 315, maxHeight))
  return {
    width, height,
    x: Math.max(12, Math.min(Number.isFinite(value.x) ? value.x! : viewport.width - width - 24, viewport.width - width - 12)),
    y: Math.max(40, Math.min(Number.isFinite(value.y) ? value.y! : 70, viewport.height - height - 12)),
  }
}

/** New screen or window: keep the full frame inside the current panel.
 *  The wider picture keeps width and shortens height. The taller picture
 *  keeps height and narrows width. Neither side grows. */
export function containComputerPreview(value: PreviewBounds, aspect: number, viewport: { width: number; height: number }): PreviewBounds {
  const ratio = Number.isFinite(aspect) && aspect > 0 ? aspect : 1
  const maxWidth = Math.max(1, viewport.width - 24)
  const maxHeight = Math.max(1, viewport.height - 70)
  const boxW = Math.min(Math.max(1, value.width), maxWidth)
  const boxH = Math.min(Math.max(1, value.height), maxHeight)
  const wider = ratio >= boxW / boxH
  let width = wider ? boxW : boxH * ratio
  let height = wider ? boxW / ratio : boxH
  if (width > maxWidth) {
    width = maxWidth
    height = width / ratio
  }
  if (height > maxHeight) {
    height = maxHeight
    width = height * ratio
  }
  return {
    width, height,
    x: Math.max(12, Math.min(value.x, viewport.width - width - 12)),
    y: Math.max(40, Math.min(value.y, viewport.height - height - 12)),
  }
}
