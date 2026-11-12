export type Suggestion = { label: string; detail: string; insert: string }

/** Who the user can address. The clerk speaks only to the Chief, so it is not listed. */
const PEOPLE: Suggestion[] = [
  { label: "@chief", detail: "coordinates the room", insert: "@chief " },
  { label: "@red", detail: "probes the target", insert: "@red " },
  { label: "@reviewer", detail: "verifies findings independently", insert: "@reviewer " },
  { label: "@reporter", detail: "writes the report", insert: "@reporter " },
]

const COMMANDS: Suggestion[] = [
  { label: "/model", detail: "choose the model each minion runs", insert: "/model" },
  { label: "/status", detail: "every lane at a glance", insert: "/status" },
  { label: "/shelf", detail: "findings on the shelf, full screen", insert: "/shelf" },
  { label: "/trajectory", detail: "the goal map, full screen", insert: "/trajectory" },
  { label: "/stop", detail: "stop a minion, or everything: /stop red, /stop all", insert: "/stop" },
  { label: "/security", detail: "back to the coding chat", insert: "/security" },
  { label: "/leave", detail: "back to the coding chat", insert: "/leave" },
]

export const KNOWN_COMMANDS = COMMANDS.map((c) => c.label)

/** What a line starting with a slash is. Anything else is a message for the Chief. */
export type Typed =
  | { kind: "message" }
  | { kind: "stop"; who: string }
  | { kind: "unknown"; word: string; near: string }

function distance(a: string, b: string): number {
  const row = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    let diagonal = row[0]!
    row[0] = i
    for (let j = 1; j <= b.length; j++) {
      const above = row[j]!
      row[j] = Math.min(above + 1, row[j - 1]! + 1, diagonal + (a[i - 1] === b[j - 1] ? 0 : 1))
      diagonal = above
    }
  }
  return row[b.length]!
}

/** A typo of a command must not reach the Chief as if it were a message. A path may. */
export function classify(text: string): Typed {
  const trimmed = text.trim()
  if (!trimmed.startsWith("/")) return { kind: "message" }
  const [word = "", ...rest] = trimmed.split(/\s+/)
  const lower = word.toLowerCase()
  if (lower.slice(1).includes("/")) return { kind: "message" }
  if (lower === "/stop") return { kind: "stop", who: (rest[0] ?? "all").replace(/^@/, "").toLowerCase() }
  if (KNOWN_COMMANDS.includes(lower)) return { kind: "message" }
  const near = KNOWN_COMMANDS.find((c) => c.startsWith(lower)) ?? KNOWN_COMMANDS.find((c) => distance(c, lower) <= 2) ?? ""
  return { kind: "unknown", word, near }
}

/** Suggestions for a draft that is still one `@name` or `/command` word. */
export function suggest(draft: string): { title: string; rows: Suggestion[] } {
  const stop = /^\/stop\s+(\S*)$/i.exec(draft)
  if (stop) {
    const word = (stop[1] ?? "").replace(/^@/, "").toLowerCase()
    const rows = [{ label: "all", detail: "every lane", insert: "/stop all" }, ...PEOPLE.filter((p) => p.label !== "@chief").map((p) => ({ label: p.label.slice(1), detail: p.detail, insert: `/stop ${p.label.slice(1)}` }))]
    return { title: " stop ", rows: rows.filter((row) => row.label.startsWith(word)) }
  }
  if (/\s/.test(draft)) return { title: "", rows: [] }
  const pool = draft.startsWith("@") ? PEOPLE : draft.startsWith("/") ? COMMANDS : []
  const word = draft.toLowerCase()
  return {
    title: draft.startsWith("@") ? " people " : " commands ",
    rows: pool.filter((row) => row.label.startsWith(word)),
  }
}
