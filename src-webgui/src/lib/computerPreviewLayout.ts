export type PreviewBounds = { x: number; y: number; width: number; height: number }

/** Fit the complete source image, with no reserved toolbar area or letterboxing. */
export function fitComputerPreview(value: Partial<PreviewBounds>, aspect: number, viewport: { width: number; height: number }): PreviewBounds {
  const ratio = Number.isFinite(aspect) && aspect > 0 ? aspect : 16 / 9
  const maxWidth = Math.max(1, viewport.width - 24)
  const maxHeight = Math.max(1, viewport.height - 70)
  const limit = Math.min(maxWidth, maxHeight * ratio)
  const minimum = Math.min(limit, Math.max(240, 120 * ratio))
  const width = Math.max(minimum, Math.min(Number.isFinite(value.width) ? value.width! : 560, limit))
  const height = width / ratio
  return {
    width, height,
    x: Math.max(12, Math.min(Number.isFinite(value.x) ? value.x! : viewport.width - width - 24, viewport.width - width - 12)),
    y: Math.max(40, Math.min(Number.isFinite(value.y) ? value.y! : 70, viewport.height - height - 12)),
  }
}
