import type { LaneRow, ModelChoice } from "./room-link.ts"

export type Minion = { avatar: string; display: string; lanes: LaneRow[] }

/** Lane rows grouped by avatar, in the order the daemon listed them. */
export function minions(rows: LaneRow[]): Minion[] {
  const out: Minion[] = []
  for (const row of rows) {
    const found = out.find((m) => m.avatar === row.avatar)
    if (found) found.lanes.push(row)
    else out.push({ avatar: row.avatar, display: row.display, lanes: [row] })
  }
  return out
}

/** Registered models whose name, id or provider contains every word typed. */
export function filterModels(choices: ModelChoice[], query: string): ModelChoice[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return choices
  return choices.filter((c) => {
    const hay = `${c.name} ${c.model_id} ${c.provider}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}
