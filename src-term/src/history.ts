/** How many recent turns the terminal mounts. In memory only — not saved. */
export const CHAT_TURNS_DEFAULT = 30
export const CHAT_TURNS_MIN = 1
export const CHAT_TURNS_MAX = 200
export const CHAT_MOUNT_TURNS = 90
/** Content budget for mounted turns, in UTF-16 units. `KOMA_TERM_WINDOW_BYTES` overrides it. */
export const CHAT_WINDOW_BYTES = positiveInt(process.env.KOMA_TERM_WINDOW_BYTES) ?? 2 * 1024 * 1024

/** Exclusive row boundary for a bounded forward window. The first turn always fits. */
export function windowEnd(rows: readonly HistoryRow[], from: number, turns = CHAT_MOUNT_TURNS, bytes = Infinity): number {
  let count = 0
  let size = 0
  for (let i = from; i < rows.length; i++) {
    if (isTurnStart(rows[i]!) && ++count > 1 && (count > turns || size > bytes)) return i
    size += rows[i]!.size ?? 0
  }
  return rows.length
}

/** Move the window start by up to `step` turns, crossing at most half the byte budget. */
export function shiftWindow(rows: readonly HistoryRow[], from: number, direction: -1 | 1, step = CHAT_TURNS_DEFAULT, bytes = Infinity): number {
  const starts = turnStarts(rows)
  let at = Math.max(0, starts.findIndex((i) => i >= from))
  let size = 0
  for (let n = 0; n < step; n++) {
    const next = at + direction
    if (next < 0 || next >= starts.length) break
    size += direction < 0 ? sizeBetween(rows, starts[next]!, starts[at]!) : sizeBetween(rows, starts[at]!, starts[next]!)
    if (n > 0 && size > bytes / 2) break
    at = next
  }
  return starts[at] ?? 0
}

const sizes = new WeakMap<object, number>()

/** Rendered weight of one message. Cached per message object, so a snapshot never rescans history. */
export function messageSize(message: SnapshotMessage): number {
  const cached = sizes.get(message)
  if (cached !== undefined) return cached
  let size = (message.content?.length ?? 0) + (message.reasoning?.length ?? 0) + (message.attachments?.length ?? 0) * 256
  for (const call of message.tool_calls ?? []) size += (call.function?.name?.length ?? 0) + (call.function?.arguments?.length ?? 0)
  sizes.set(message, size)
  return size
}

function turnStarts(rows: readonly HistoryRow[]): number[] {
  const starts: number[] = []
  for (let i = 0; i < rows.length; i++) if (isTurnStart(rows[i]!)) starts.push(i)
  return starts
}

function sizeBetween(rows: readonly HistoryRow[], from: number, to: number): number {
  let size = 0
  for (let i = from; i < to; i++) size += rows[i]!.size ?? 0
  return size
}

function positiveInt(raw: string | undefined): number | null {
  const n = Number(raw)
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : null
}

const SHELL_MARK = "\u2064"
const BASH_NUDGE_MARK = "\u2061"
const EXT_PROMPT_MARK = "\u2060\u2060"

export type HistoryRow = {
  role: string
  content: string
  /** Index in the daemon snapshot. Stable across an append-only refresh. */
  sourceIndex: number
  /** Content weight of this row plus the tool results that follow it. */
  size?: number
}

type SnapshotMessage = {
  role: string
  content?: string
  reasoning?: string
  attachments?: readonly unknown[]
  tool_calls?: readonly { function?: { name?: string; arguments?: string } }[]
}

/** Rows the transcript can draw. Tool results stay on the assistant card. */
export function historyRows(messages: readonly SnapshotMessage[]): HistoryRow[] {
  const rows: HistoryRow[] = []
  messages.forEach((message, sourceIndex) => {
    if (message.role === "system") return
    if (message.role === "tool") {
      const owner = rows[rows.length - 1]
      if (owner) owner.size = (owner.size ?? 0) + messageSize(message)
      return
    }
    rows.push({ role: message.role, content: message.content ?? "", sourceIndex, size: messageSize(message) })
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

/** First row to mount so the slice holds at most `turns` prompts and, past the newest turn, `bytes` of content. */
export function visibleFromTurn(rows: readonly HistoryRow[], turns: number, bytes = Infinity): number {
  const want = Math.max(CHAT_TURNS_MIN, Math.floor(turns) || CHAT_TURNS_MIN)
  const starts = turnStarts(rows)
  let end = rows.length
  let size = 0
  for (let k = starts.length - 1; k >= 0; k--) {
    const taken = starts.length - 1 - k
    const turn = sizeBetween(rows, starts[k]!, end)
    if (taken > 0 && (taken >= want || size + turn > bytes)) return starts[k + 1]!
    size += turn
    end = starts[k]!
  }
  return 0
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
  bytes = Infinity,
): number {
  if (sessionChanged || stick || next.length === 0) return visibleFromTurn(next, turns, bytes)
  if (extraTurns > 0 && isHistoryPrepend(prev, next)) return visibleFromTurn(next, turns + extraTurns, bytes)
  const anchor = prev[from]
  if (anchor == null) return Math.min(Math.max(0, from), Math.max(0, next.length - 1))
  const found = next.findIndex((row) => row.sourceIndex === anchor.sourceIndex)
  if (found >= 0) return found
  return visibleFromTurn(next, turns, bytes)
}
