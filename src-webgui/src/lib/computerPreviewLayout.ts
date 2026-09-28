export type PreviewBounds = { x: number; y: number; width: number; height: number }

/** Fit a shared image; before selection, width and height are independent. */
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
