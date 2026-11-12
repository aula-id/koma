import type { EventInfo, LaneInfo } from "./room-link.ts"

export type Tone = "success" | "info" | "warn" | "error" | "dim"

/** A shape-carrying glyph and a tone per phase, so the table reads without colour. */
export function phaseMark(phase: string): { glyph: string; tone: Tone } {
  switch (phase) {
    case "running": return { glyph: "●", tone: "info" }
    case "paused": return { glyph: "‖", tone: "warn" }
    case "submitted": return { glyph: "✔", tone: "success" }
    case "done": return { glyph: "✔", tone: "success" }
    case "failed": return { glyph: "✕", tone: "error" }
    case "stopped": return { glyph: "■", tone: "warn" }
    default: return { glyph: "○", tone: "dim" }
  }
}

export type Standing = { alias: string; name: string; state: string; glyph: string; tone: Tone; spinning: boolean }

/** The short tag of an avatar: its alias, or the first two letters of its name. */
function tagOf(row: LaneInfo): string {
  return (row.alias || (row.display || row.avatar).replace(/[^A-Za-z0-9]/g, "").slice(0, 2)).toUpperCase()
}

/** One entry per avatar for the strip under the composer: is it thinking or ready. */
export function standing(rows: LaneInfo[]): Standing[] {
  const order: string[] = []
  const seen = new Map<string, { row: LaneInfo; phases: string[] }>()
  for (const row of rows) {
    if (!seen.has(row.avatar)) {
      order.push(row.avatar)
      seen.set(row.avatar, { row, phases: [] })
    }
    seen.get(row.avatar)!.phases.push(row.phase)
  }
  return order.map((avatar) => {
    const { row, phases } = seen.get(avatar)!
    const pick = (...wanted: string[]) => wanted.find((w) => phases.includes(w))
    const phase = pick("running") ?? pick("paused") ?? pick("failed") ?? pick("stopped") ?? "ready"
    const state = phase === "running" ? "thinking" : phase
    const mark = phase === "ready" ? { glyph: "○", tone: "dim" as Tone } : phaseMark(phase)
    return { alias: tagOf(row), name: row.display || row.avatar, state, ...mark, spinning: phase === "running" }
  })
}

/** How long ago, in the largest whole unit. Empty when the time is unknown. */
export function ago(nowMs: number, atMs: number): string {
  if (!atMs || !nowMs) return ""
  const seconds = Math.max(0, Math.floor((nowMs - atMs) / 1000))
  if (seconds < 2) return "now"
  if (seconds < 90) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 90) return `${minutes}m`
  return `${Math.floor(minutes / 60)}h`
}

export function clip(text: string, width: number): string {
  const chars = [...text]
  if (width <= 0) return ""
  return chars.length <= width ? text : `${chars.slice(0, Math.max(0, width - 1)).join("")}…`
}

export type LaneCells = { glyph: string; tone: Tone; name: string; lane: string; phase: string; task: string; steps: string; tool: string; age: string }

const NAME = 10
const LANE = 5
const PHASE = 9
const STEPS = 5
const TOOL = 14
const AGE = 5

/** Column widths for a row of `width` characters. The task takes what the fixed columns leave. */
export function laneWidths(width: number): { task: number; tool: number } {
  const fixed = 2 + NAME + 1 + LANE + 1 + PHASE + 1 + STEPS + 1 + AGE + 1
  const roomy = width - fixed
  const tool = roomy >= 50 ? TOOL : 0
  return { task: Math.max(8, roomy - (tool ? tool + 1 : 0)), tool }
}

const pad = (text: string, width: number) => clip(text, width).padEnd(width)

/** One table row, cut to the columns that fit. */
export function laneCells(row: LaneInfo, nowMs: number, width: number): LaneCells {
  const { glyph, tone } = phaseMark(row.phase)
  const cols = laneWidths(width)
  const task = row.error ? `✕ ${row.error}` : row.task
  return {
    glyph,
    tone,
    name: pad(row.display || row.avatar, NAME),
    lane: pad(row.lane, LANE),
    phase: pad(row.phase, PHASE),
    task: pad(task, cols.task),
    steps: row.steps ? `${row.steps}`.padStart(STEPS - 1).padEnd(STEPS) : " ".repeat(STEPS),
    tool: cols.tool ? pad(row.last_tool, cols.tool) : "",
    age: ago(nowMs, row.last_event_ms).padStart(AGE),
  }
}

/** Column titles aligned with [`laneCells`]. */
export function laneHeader(width: number): string {
  const cols = laneWidths(width)
  const parts = ["  ", pad("lane", NAME), " ", pad("", LANE), " ", pad("state", PHASE), " ", pad("doing", cols.task), " ", pad("steps", STEPS), cols.tool ? ` ${pad("last tool", cols.tool)}` : "", " ", "age".padStart(AGE)]
  return parts.join("")
}

/** Who is busy, for the panel's corner. */
export function busySummary(rows: LaneInfo[]): string {
  const running = rows.filter((row) => row.phase === "running").length
  const idle = rows.filter((row) => row.phase === "idle").length
  return `${running} running · ${idle} idle`
}

export type EventCells = { age: string; avatar: string; marker: string; label: string }

export function eventCells(event: EventInfo, nowMs: number): EventCells {
  return { age: ago(nowMs, event.at_ms).padStart(5), avatar: event.avatar.padEnd(10), marker: event.marker, label: event.label }
}
