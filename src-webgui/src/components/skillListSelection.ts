export type SkillSelectionModifiers = {
  ctrlKey?: boolean
  metaKey?: boolean
  shiftKey?: boolean
}

export type SkillSelectionState = { selected: string[]; anchor: string | null }

/** Session-list parity selection over the currently visible opaque skill ids. */
export function nextSkillSelection(
  visible: string[],
  current: string[],
  anchor: string | null,
  picked: string,
  modifiers: SkillSelectionModifiers = {},
): SkillSelectionState {
  if (!visible.includes(picked)) return { selected: current, anchor }
  if (modifiers.shiftKey && anchor) {
    const from = visible.indexOf(anchor)
    const to = visible.indexOf(picked)
    if (from >= 0 && to >= 0) {
      const [start, end] = from <= to ? [from, to] : [to, from]
      return { selected: visible.slice(start, end + 1), anchor }
    }
  }
  if (modifiers.ctrlKey || modifiers.metaKey) {
    const next = new Set(current)
    if (next.has(picked)) next.delete(picked)
    else next.add(picked)
    return { selected: visible.filter((id) => next.has(id)), anchor: picked }
  }
  return { selected: [picked], anchor: picked }
}
