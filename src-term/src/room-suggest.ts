export type Suggestion = { label: string; detail: string; insert: string }

/** Who the user can address. The clerk speaks only to the Chief, so it is not listed. */
const PEOPLE: Suggestion[] = [
  { label: "@chief", detail: "coordinates the room", insert: "@chief " },
  { label: "@red", detail: "probes the target", insert: "@red " },
  { label: "@reviewer", detail: "verifies findings independently", insert: "@reviewer " },
  { label: "@reporter", detail: "writes the report", insert: "@reporter " },
]

const COMMANDS: Suggestion[] = [
  { label: "/security", detail: "back to the coding chat", insert: "/security" },
  { label: "/leave", detail: "back to the coding chat", insert: "/leave" },
]

/** Suggestions for a draft that is still one `@name` or `/command` word. */
export function suggest(draft: string): { title: string; rows: Suggestion[] } {
  if (/\s/.test(draft)) return { title: "", rows: [] }
  const pool = draft.startsWith("@") ? PEOPLE : draft.startsWith("/") ? COMMANDS : []
  const word = draft.toLowerCase()
  return {
    title: draft.startsWith("@") ? " people " : " commands ",
    rows: pool.filter((row) => row.label.startsWith(word)),
  }
}
