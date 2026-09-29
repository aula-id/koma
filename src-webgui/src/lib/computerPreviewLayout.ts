export type PreviewBounds = { x: number; y: number; width: number; height: number }

/** Keep the box the user sized. Width and height stay independent.
 *  A new frame does not change them. The viewport can only shrink a side
 *  that would otherwise leave the screen; the picture letterboxes inside. */
export function fitComputerPreview(value: Partial<PreviewBounds>, viewport: { width: number; height: number }): PreviewBounds {
  const maxWidth = Math.max(1, viewport.width - 24)
  const maxHeight = Math.max(1, viewport.height - 70)
  const width = Math.min(Math.max(1, Number.isFinite(value.width) ? value.width! : 560), maxWidth)
  const height = Math.min(Math.max(1, Number.isFinite(value.height) ? value.height! : 315), maxHeight)
  return {
    width, height,
    x: Math.max(12, Math.min(Number.isFinite(value.x) ? value.x! : viewport.width - width - 24, Math.max(12, viewport.width - width - 12))),
    y: Math.max(40, Math.min(Number.isFinite(value.y) ? value.y! : 70, Math.max(40, viewport.height - height - 12))),
  }
}
