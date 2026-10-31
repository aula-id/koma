/** How many recent turns the terminal mounts. In memory only — not saved. */
export const CHAT_TURNS_DEFAULT = 30
export const CHAT_TURNS_MIN = 1
export const CHAT_TURNS_MAX = 200
export const CHAT_MOUNT_TURNS = 90

/** Exclusive row boundary for a bounded forward window. */
export function windowEnd(rows: readonly HistoryRow[], from: number, turns = CHAT_MOUNT_TURNS): number {
  let count = 0
  for (let i = from; i < rows.length; i++) {
    if (isTurnStart(rows[i]) && ++count > turns) return i
  }
  return rows.length
}

export function shiftWindow(rows: readonly HistoryRow[], from: number, direction: -1 | 1, step = CHAT_TURNS_DEFAULT): number {
  const starts = rows.flatMap((row, i) => isTurnStart(row) ? [i] : [])
  const at = Math.max(0, starts.findIndex((i) => i >= from))
  return starts[Math.max(0, Math.min(starts.length - 1, at + direction * step))] ?? 0
}

const SHELL_MARK = "\u2064"
const BASH_NUDGE_MARK = "\u2061"
const EXT_PROMPT_MARK = "\u2060\u2060"

export type HistoryRow = {
  role: string
  content: string
  /** Index in the daemon snapshot. Stable across an append-only refresh. */
  sourceIndex: number
}

type SnapshotMessage = { role: string; content?: string }

/** Rows the transcript can draw. Tool results stay on the assistant card. */
export function historyRows(messages: readonly SnapshotMessage[]): HistoryRow[] {
  const rows: HistoryRow[] = []
  messages.forEach((message, sourceIndex) => {
    if (message.role === "system" || message.role === "tool") return
    rows.push({ role: message.role, content: message.content ?? "", sourceIndex })
  })
  return rows
}

/** A turn starts at a plain user prompt. Shell lines and bash nudges stay with the turn they follow. */
export function isTurnStart(row: HistoryRow): boolean {
  if (row.role !== "user") return false
  const content = row.content
  if (content.startsWith(SHELL_MARK) || content.startsWith(BASH_NUDGE_MARK) || content.startsWith(EXT_PROMPT_MARK)) {
    return false
  }
  return true
}

export function countTurns(rows: readonly HistoryRow[]): number {
  let n = 0
  for (const row of rows) if (isTurnStart(row)) n++
  return n
}

export function clampChatTurns(n: number): number {
  if (!Number.isFinite(n)) return CHAT_TURNS_DEFAULT
  return Math.min(CHAT_TURNS_MAX, Math.max(CHAT_TURNS_MIN, Math.floor(n)))
}

/** First row to mount so the slice holds at most `turns` prompts. */
export function visibleFromTurn(rows: readonly HistoryRow[], turns: number): number {
  const want = Math.max(CHAT_TURNS_MIN, Math.floor(turns) || CHAT_TURNS_MIN)
  const starts: number[] = []
  for (let i = 0; i < rows.length; i++) {
    if (isTurnStart(rows[i])) starts.push(i)
  }
  if (starts.length <= want) return 0
  return starts[starts.length - want]
}

/** Reveal `step` more turns above `from`. Stops at the oldest loaded row. */
export function pageFromTurn(rows: readonly HistoryRow[], from: number, step: number): number {
  const mounted = countTurns(rows.slice(Math.max(0, from)))
  return visibleFromTurn(rows, mounted + Math.max(CHAT_TURNS_MIN, Math.floor(step) || CHAT_TURNS_MIN))
}

export function isHistoryPrepend(prev: readonly HistoryRow[], next: readonly HistoryRow[]): boolean {
  if (prev.length === 0 || next.length <= prev.length) return false
  const shift = next.length - prev.length
  return next[shift]?.sourceIndex === prev[0]?.sourceIndex
}

/**
 * Next mount index when the transcript array is replaced.
 * Following the tail snaps to the last `turns` prompts.
 * A page the reader asked for keeps those extra turns.
 * Any other update keeps the anchor row.
 */
export function nextRenderFrom(
  prev: readonly HistoryRow[],
  next: readonly HistoryRow[],
  from: number,
  stick: boolean,
  turns: number,
  sessionChanged: boolean,
  extraTurns = 0,
): number {
  if (sessionChanged || stick || next.length === 0) return visibleFromTurn(next, turns)
  if (extraTurns > 0 && isHistoryPrepend(prev, next)) return visibleFromTurn(next, turns + extraTurns)
  const anchor = prev[from]
  if (anchor == null) return Math.min(Math.max(0, from), Math.max(0, next.length - 1))
  const found = next.findIndex((row) => row.sourceIndex === anchor.sourceIndex)
  if (found >= 0) return found
  return visibleFromTurn(next, turns)
}
