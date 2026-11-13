/**
 * Raw JSON in room text (an assignment prompt that carries claim and evidence references, a
 * tool payload) is hard to read as one run of braces. These helpers find JSON inside prose and
 * set it out as a nested list.
 */

const REV = "revision"
const HEAD = ["key", "claim"]
const SKIP_LEN = 24

function scalar(value: unknown): string {
  if (typeof value === "string") return value.includes("/") && !value.includes(" ") ? `\`${value}\`` : value
  return String(value)
}

function labelOf(item: Record<string, unknown>): { label: string; used: string[] } | null {
  for (const name of HEAD) {
    const v = item[name]
    if (typeof v === "string") {
      const rev = item[REV]
      return { label: `${scalar(v)}${rev !== undefined && typeof rev !== "object" ? ` (rev ${rev})` : ""}`, used: [name, REV] }
    }
  }
  return null
}

function lines(value: unknown, depth: number, out: string[]): void {
  const pad = "  ".repeat(depth)
  if (Array.isArray(value)) {
    for (const item of value) {
      if (item === null || typeof item !== "object") { out.push(`${pad}- ${scalar(item)}`); continue }
      if (Array.isArray(item)) { out.push(`${pad}-`); lines(item, depth + 1, out); continue }
      const head = labelOf(item as Record<string, unknown>)
      if (head) {
        out.push(`${pad}- ${head.label}`)
        lines(Object.fromEntries(Object.entries(item).filter(([k]) => !head.used.includes(k))), depth + 1, out)
      } else {
        const entries = Object.entries(item)
        const [first, ...rest] = entries
        if (!first) { out.push(`${pad}- {}`); continue }
        if (first[1] !== null && typeof first[1] === "object") { out.push(`${pad}- **${first[0]}**`); lines(first[1], depth + 1, out) }
        else out.push(`${pad}- **${first[0]}**: ${scalar(first[1])}`)
        lines(Object.fromEntries(rest), depth + 1, out)
      }
    }
    return
  }
  if (value !== null && typeof value === "object") {
    for (const [name, v] of Object.entries(value)) {
      if (v !== null && typeof v === "object") {
        if (Array.isArray(v) && v.length === 0) { out.push(`${pad}- **${name}**: none`); continue }
        out.push(`${pad}- **${name}**`)
        lines(v, depth + 1, out)
      } else out.push(`${pad}- **${name}**: ${scalar(v)}`)
    }
    return
  }
  out.push(`${pad}- ${scalar(value)}`)
}

/** A parsed JSON value as a markdown list. */
export function outline(value: unknown): string {
  const out: string[] = []
  lines(value, 0, out)
  return out.join("\n")
}

/** The end of the JSON block that opens at `from`, or -1. Brackets inside strings do not count. */
function blockEnd(text: string, from: number): number {
  let depth = 0
  let quoted = false
  for (let i = from; i < text.length; i++) {
    const c = text[i]
    if (quoted) {
      if (c === "\\") i++
      else if (c === '"') quoted = false
      continue
    }
    if (c === '"') quoted = true
    else if (c === "[" || c === "{") depth++
    else if (c === "]" || c === "}") { depth--; if (depth === 0) return i + 1 }
  }
  return -1
}

function prettyProse(text: string): string {
  let out = ""
  let at = 0
  while (at < text.length) {
    const next = text.slice(at).search(/[[{]/)
    if (next < 0) { out += text.slice(at); break }
    const start = at + next
    const end = blockEnd(text, start)
    let shown: string | null = null
    if (end > 0 && end - start >= SKIP_LEN) {
      try {
        const value = JSON.parse(text.slice(start, end))
        if (value !== null && typeof value === "object") shown = `\n\n${outline(value)}\n\n`
      } catch {
        // Not JSON after all: leave it as it was.
      }
    }
    out += text.slice(at, start)
    if (shown !== null) { out += shown; at = end } else { out += text[start]; at = start + 1 }
  }
  return out
}

/** Text with any JSON block set out as a list. Code fences are left alone. */
export function prettyText(text: string): string {
  if (!/[[{]/.test(text)) return text
  return text.split(/(```[\s\S]*?```)/g).map((part, i) => i % 2 ? part : prettyProse(part)).join("")
}

/** A card body for the room's own tools: what was asked, in words and lists. Null leaves the raw payload. */
export function toolDigest(name: string, args: string): string | null {
  if (!name.startsWith("group_")) return null
  let parsed: unknown
  try { parsed = JSON.parse(args || "{}") } catch { return null }
  if (parsed === null || typeof parsed !== "object") return null
  const record = parsed as Record<string, unknown>
  const parts: string[] = []
  for (const [k, v] of Object.entries(record)) {
    if (k === "assignments" && Array.isArray(v)) {
      for (const a of v) {
        if (a === null || typeof a !== "object") continue
        const { avatar, kind, prompt, ...rest } = a as Record<string, unknown>
        parts.push(`**${String(avatar ?? "")}** · ${String(kind ?? "")}`)
        if (typeof prompt === "string") parts.push(prettyText(prompt))
        if (Object.keys(rest).length) parts.push(outline(rest))
      }
    } else if (typeof v === "string") parts.push(k === "goal" || k === "report" || k === "note" ? prettyText(v) : `**${k}**: ${v}`)
    else if (v !== null && typeof v === "object") parts.push(`**${k}**\n${outline(v)}`)
    else parts.push(`**${k}**: ${String(v)}`)
  }
  return parts.join("\n\n") || null
}
