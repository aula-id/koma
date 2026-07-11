// Single source of truth for "author -> colour": both AuthorAvatar (the
// commit-row/detail-pane badge) and GraphBubble (GK5b's bubble/activity
// chart) must render the SAME author with the SAME hue, or the chart's
// legend/bubbles and the graph's avatars would silently disagree. Extracted
// out of AuthorAvatar.tsx rather than duplicated.

import { LANE_COLORS } from './gitGraphLayout'

// Small, deliberately-deterministic string hash (djb2-ish) — only used to
// pick a stable palette index, never for anything security-sensitive.
function hashStr(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

// Deterministic author -> colour, keyed by email (falling back to name, then
// a literal "?" so an entirely-blank author still gets a stable colour).
// Reuses the graph's own `LANE_COLORS` palette (gitGraphLayout.ts) — already
// chosen to read on both the light and dark koma themes — rather than a
// bespoke second copy of the same set of hues.
export function authorColor(name: string, email: string): string {
  const key = email.trim() || name.trim() || '?'
  return LANE_COLORS[hashStr(key) % LANE_COLORS.length]
}
