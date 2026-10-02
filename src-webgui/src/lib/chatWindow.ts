// How many recent turns the chat mounts. In memory only — not a host pref.
export const CHAT_TURNS_DEFAULT = 30
export const CHAT_TURNS_MIN = 1
export const CHAT_TURNS_MAX = 200

export type TurnRow = {
  role: string
  kind?: string | null
  idx?: number
}

// A turn starts at a plain user prompt. Shell lines and bash nudges stay with
// the turn they follow. Assistant and tool rows belong to the prompt before them.
export function isTurnStart(row: TurnRow): boolean {
  return row.role === 'user' && (row.kind == null || row.kind === '')
}

export function countTurns(rows: readonly TurnRow[]): number {
  let n = 0
  for (const row of rows) if (isTurnStart(row)) n++
  return n
}

export function clampChatTurns(n: number): number {
  if (!Number.isFinite(n)) return CHAT_TURNS_DEFAULT
  return Math.min(CHAT_TURNS_MAX, Math.max(CHAT_TURNS_MIN, Math.floor(n)))
}

// First row to mount so the slice holds at most `turns` prompts.
// Fewer prompts than requested mounts the whole loaded transcript, including
// rows that sit before the first prompt (they belong to that first turn).
export function visibleFromTurn(rows: readonly TurnRow[], turns: number): number {
  const want = Math.max(CHAT_TURNS_MIN, Math.floor(turns) || CHAT_TURNS_MIN)
  const starts: number[] = []
  for (let i = 0; i < rows.length; i++) {
    if (isTurnStart(rows[i])) starts.push(i)
  }
  if (starts.length <= want) return 0
  return starts[starts.length - want]
}

// Reveal `step` more turns above `from`. Stops at the oldest loaded row.
export function pageFromTurn(rows: readonly TurnRow[], from: number, step: number): number {
  const mounted = countTurns(rows.slice(Math.max(0, from)))
  return visibleFromTurn(rows, mounted + Math.max(CHAT_TURNS_MIN, Math.floor(step) || CHAT_TURNS_MIN))
}

// True when `next` is `prev` with older rows inserted at the front and nothing
// appended. A tail append, or a snapshot that grows both ends, is not a page
// the reader asked for — those keep the anchor row.
export function isHistoryPrepend(prev: readonly TurnRow[], next: readonly TurnRow[]): boolean {
  if (prev.length === 0 || next.length <= prev.length) return false
  const shift = next.length - prev.length
  const oldFirst = prev[0]
  if (typeof oldFirst.idx === 'number') return next[shift]?.idx === oldFirst.idx
  return next[shift] === oldFirst
}

// Next mount index when the transcript array is replaced.
// Following the tail snaps to the last `turns` prompts.
// A history page the reader asked for (`extraTurns` > 0) opens that many more.
// Any other update keeps the anchor row so the lines on screen stay put.
export function nextRenderFrom(
  prev: readonly TurnRow[],
  next: readonly TurnRow[],
  from: number,
  stick: boolean,
  turns: number,
  sessionChanged: boolean,
  extraTurns = 0,
): number {
  if (sessionChanged || stick || next.length === 0) return visibleFromTurn(next, turns)
  if (extraTurns > 0 && isHistoryPrepend(prev, next)) return visibleFromTurn(next, turns + extraTurns)
  const anchor = prev[from]
  if (anchor == null || typeof anchor.idx !== 'number') {
    if (from >= 0 && from < next.length && next[from] === anchor) return from
    return Math.min(Math.max(0, from), Math.max(0, next.length - 1))
  }
  const found = next.findIndex((row) => row.idx === anchor.idx)
  if (found >= 0) return found
  return visibleFromTurn(next, turns)
}
