import { describe, expect, test } from "bun:test"
import {
  CHAT_TURNS_DEFAULT,
  countTurns,
  historyRows,
  isHistoryPrepend,
  isTurnStart,
  messageSize,
  nextRenderFrom,
  pageFromTurn,
  shiftWindow,
  visibleFromTurn,
  windowEnd,
  type HistoryRow,
} from "./history.ts"

const prompt = (sourceIndex: number): HistoryRow => ({ role: "user", content: "hi", sourceIndex })
const reply = (sourceIndex: number): HistoryRow => ({ role: "assistant", content: "ok", sourceIndex })
const shell = (sourceIndex: number): HistoryRow => ({ role: "user", content: "\u2064$ ls", sourceIndex })

function transcript(turns: number, lead = 0): HistoryRow[] {
  const rows: HistoryRow[] = []
  for (let i = 0; i < lead; i++) rows.push(reply(i))
  for (let t = 0; t < turns; t++) {
    const base = lead + t * 3
    rows.push(prompt(base), reply(base + 1), shell(base + 2))
  }
  return rows
}

describe("chat history window", () => {
  test("matches the GUI default of 30 turns", () => {
    expect(CHAT_TURNS_DEFAULT).toBe(30)
    const few = transcript(4)
    expect(visibleFromTurn(few, CHAT_TURNS_DEFAULT)).toBe(0)
    expect(countTurns(few)).toBe(4)
    expect(isTurnStart(shell(1))).toBe(false)
  })

  test("keeps rows that sit before the first of the last 30 prompts", () => {
    const exact = transcript(CHAT_TURNS_DEFAULT, 2)
    expect(visibleFromTurn(exact, 30)).toBe(0)
    expect(exact[0]?.role).toBe("assistant")
  })

  test("mounts exactly the last 30 prompts and pages 30 more", () => {
    const more = transcript(31)
    const from30 = visibleFromTurn(more, 30)
    expect(more[from30]?.role).toBe("user")
    expect(countTurns(more.slice(from30))).toBe(30)
    expect(countTurns(more.slice(0, from30))).toBe(1)

    const eighty = transcript(80)
    const tail = visibleFromTurn(eighty, 30)
    const older = pageFromTurn(eighty, tail, 30)
    expect(countTurns(eighty.slice(tail))).toBe(30)
    expect(countTurns(eighty.slice(older))).toBe(60)
    expect(older).toBeLessThan(tail)
  })

  test("a new reply while reading keeps the anchor row", () => {
    const eighty = transcript(80)
    const tail = visibleFromTurn(eighty, 30)
    const anchor = eighty[tail]
    const appended = [...eighty, prompt(10_000), reply(10_001)]
    expect(nextRenderFrom(eighty, appended, tail, false, 30, false, 30)).toBe(
      appended.findIndex((row) => row.sourceIndex === anchor?.sourceIndex),
    )
    expect(isHistoryPrepend(eighty, appended)).toBe(false)
    expect(nextRenderFrom(eighty, appended, tail, true, 30, false)).toBe(visibleFromTurn(appended, 30))
  })

  test("drops system and tool rows before counting turns", () => {
    const rows = historyRows([
      { role: "system", content: "hidden" },
      { role: "user", content: "one" },
      { role: "assistant", content: "two" },
      { role: "tool", content: "result" },
      { role: "user", content: "\u2061bash done" },
    ])
    expect(rows.map((row) => row.role)).toEqual(["user", "assistant", "user"])
    expect(countTurns(rows)).toBe(1)
  })
})

describe("byte-budgeted window", () => {
  const sized = (sizes: number[]): HistoryRow[] =>
    sizes.flatMap((size, t) => [
      { role: "user", content: "hi", sourceIndex: t * 2, size: 2 },
      { role: "assistant", content: "", sourceIndex: t * 2 + 1, size },
    ])

  test("tool results count toward the assistant row before them and sizes are cached per message", () => {
    const tool = { role: "tool", content: "x".repeat(500) }
    const messages = [{ role: "user", content: "hi" }, { role: "assistant", content: "ok", tool_calls: [{ function: { name: "run", arguments: "{}" } }] }, tool]
    const rows = historyRows(messages)
    expect(rows[1]?.size).toBe(2 + 3 + 2 + 500)
    expect(messageSize(tool)).toBe(500)
    tool.content = "changed"
    expect(messageSize(tool)).toBe(500)
  })

  test("one oversized turn still mounts alone", () => {
    const rows = sized([10, 5_000_000, 10])
    expect(windowEnd(rows, 2, 90, 1000)).toBe(4)
    expect(visibleFromTurn(sized([10, 10, 5_000_000]), 30, 1000)).toBe(4)
  })

  test("small turns stop at the turn cap and mixed sizes stop at the byte budget", () => {
    const small = sized(Array(200).fill(10))
    expect(countTurns(small.slice(0, windowEnd(small, 0, 90, 1_000_000)))).toBe(90)
    const mixed = sized([300, 300, 300, 300, 300])
    expect(countTurns(mixed.slice(0, windowEnd(mixed, 0, 90, 700)))).toBe(3)
    expect(countTurns(mixed.slice(visibleFromTurn(mixed, 30, 700)))).toBe(2)
    expect(visibleFromTurn(mixed, 30)).toBe(0)
  })

  test("paging crosses at most half the budget but always moves one turn", () => {
    const rows = sized(Array(50).fill(100))
    const tail = visibleFromTurn(rows, 30)
    expect(countTurns(rows.slice(shiftWindow(rows, tail, -1, 30, 1000), tail))).toBe(4)
    expect(countTurns(rows.slice(shiftWindow(rows, tail, -1, 30), tail))).toBe(20)
    const huge = sized([5_000_000, 5_000_000, 5_000_000])
    expect(shiftWindow(huge, 4, -1, 30, 1000)).toBe(2)
  })
})