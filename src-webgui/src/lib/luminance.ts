// Relative (perceptual) luminance of a `#rrggbb` hex color, ITU-R BT.709
// coefficients. Shared by anything that needs to pick a light/dark variant
// (Monaco theme base in DiffTab, Shiki code-block token theme in komaShiki/
// MessageBody) off the live koma palette background — keeps the "what counts
// as light vs dark" formula in exactly one place.
export function luminance(hex: string): number {
  const h = hex.replace('#', '')
  if (h.length < 6) return 0
  const r = parseInt(h.slice(0, 2), 16) / 255
  const g = parseInt(h.slice(2, 4), 16) / 255
  const b = parseInt(h.slice(4, 6), 16) / 255
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}
