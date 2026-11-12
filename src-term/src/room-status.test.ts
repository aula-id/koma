import { expect, test } from "bun:test"
import { ago, busySummary, clip, eventCells, laneCells, laneHeader, laneWidths, phaseMark } from "./room-status.ts"
import type { LaneInfo } from "./room-link.ts"

const lane = (over: Partial<LaneInfo>): LaneInfo => ({ avatar: "red", display: "Red", role: "red", lane: "work", phase: "running", task: "probe: check auth", started_ms: 0, last_event_ms: 0, steps: 0, last_tool: "", error: "", ...over })

test("phases carry a glyph that reads without colour", () => {
  expect(phaseMark("running").glyph).toBe("●")
  expect(phaseMark("idle").glyph).toBe("○")
  expect(phaseMark("failed").glyph).toBe("✕")
  expect(phaseMark("submitted").tone).toBe("success")
  expect(phaseMark("paused").tone).toBe("warn")
})

test("ago uses the largest whole unit and stays empty when the time is unknown", () => {
  expect(ago(100_000, 99_500)).toBe("now")
  expect(ago(100_000, 70_000)).toBe("30s")
  expect(ago(1_000_000, 700_000)).toBe("5m")
  expect(ago(100_000_000, 80_000_000)).toBe("5h")
  expect(ago(1000, 0)).toBe("")
})

test("a lane row has fixed columns, a task that takes the rest, and a failure shown in place of it", () => {
  const wide = laneCells(lane({ steps: 12, last_tool: "grep", last_event_ms: 98_000 }), 100_000, 120)
  expect(wide.name).toBe("Red       ")
  expect(wide.tool.trim()).toBe("grep")
  expect(wide.age.trim()).toBe("2s")
  expect(wide.steps.trim()).toBe("12")
  expect([...wide.task].length).toBe(laneWidths(120).task)
  const narrow = laneCells(lane({}), 0, 60)
  expect(narrow.tool).toBe("")
  expect(laneCells(lane({ phase: "failed", error: "model refused" }), 0, 100).task.startsWith("✕ model refused")).toBe(true)
  expect(laneHeader(120)).toContain("last tool")
  expect(laneHeader(60)).not.toContain("last tool")
})

test("the summary and the events read plainly", () => {
  expect(busySummary([lane({}), lane({ phase: "idle" }), lane({ phase: "idle" })])).toBe("1 running · 2 idle")
  expect(eventCells({ at_ms: 95_000, avatar: "red", marker: "○", label: "claims/a" }, 100_000)).toEqual({ age: "   5s", avatar: "red       ", marker: "○", label: "claims/a" })
  expect(clip("abcdef", 4)).toBe("abc…")
})

import { standing } from "./room-status.ts"

test("each avatar reads thinking or ready, whichever of its lanes is busy", () => {
  const row = (avatar: string, lane: string, phase: string) => ({ avatar, display: avatar, role: "", lane, phase, task: "", started_ms: 0, last_event_ms: 0, steps: 0, last_tool: "", error: "" })
  const result = standing([
    row("chief", "chat", "running"),
    row("red", "chat", "idle"), row("red", "work", "running"),
    row("reviewer", "", "idle"),
    row("reporter", "work", "done"),
    row("clerk", "work", "stopped"),
  ])
  expect(result.map((s) => `${s.name}:${s.state}`)).toEqual(["chief:thinking", "red:thinking", "reviewer:ready", "reporter:ready", "clerk:stopped"])
})
