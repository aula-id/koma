import type { LanePart, RoomFrame } from "./room-link.ts"

/** One piece of a message, in the order the lane produced it. */
export type Item =
  | { kind: "text"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; id: string; name: string; args: string; result?: { ok: boolean; text: string } }

/**
 * One message in the room. A lane's consecutive pieces fold into one message, so a turn
 * with thinking, tool calls and a reply reads as one block, not a line per model step.
 */
export type Line = { id: string; author: string; lane?: string; items: Item[] }

const MERGE_LIMIT = 200

export function say(author: string, text: string, lane?: string): Line {
  return { id: crypto.randomUUID(), author, lane, items: [{ kind: "text", text }] }
}

/** True for a message the user typed. */
export function isYou(line: Line): boolean {
  return line.author === "you"
}

/** Plain text of a message, for search and tests. */
export function plain(line: Line): string {
  return line.items.map((item) => item.kind === "tool" ? `${item.name} ${item.args}` : item.text).join("\n")
}

function append(lines: Line[], author: string, lane: string, item: Item): Line[] {
  const last = lines[lines.length - 1]
  if (last && last.author === author && last.lane === lane && !isYou(last)) {
    const items = [...last.items]
    const tail = items[items.length - 1]
    // Text that arrives right after text of the same turn joins it.
    if (item.kind === "text" && tail?.kind === "text") items[items.length - 1] = { kind: "text", text: `${tail.text}\n\n${item.text}` }
    else items.push(item)
    return [...lines.slice(0, -1), { ...last, items }]
  }
  return [...lines, { id: crypto.randomUUID(), author, lane, items: [item] }]
}

function setResult(lines: Line[], part: Extract<LanePart, { kind: "tool_result" }>): Line[] {
  for (let i = lines.length - 1; i >= Math.max(0, lines.length - MERGE_LIMIT); i--) {
    const line = lines[i]
    if (!line) continue
    const at = line.items.findIndex((item) => item.kind === "tool" && item.id === part.id)
    if (at < 0) continue
    const items = [...line.items]
    const found = items[at]
    if (found?.kind === "tool") items[at] = { ...found, result: { ok: part.ok, text: part.result } }
    return [...lines.slice(0, i), { ...line, items }, ...lines.slice(i + 1)]
  }
  return lines
}

/** Fold one daemon frame into the message list. Frames that are not messages leave it unchanged. */
export function reduceLines(lines: Line[], frame: RoomFrame): Line[] {
  switch (frame.type) {
    case "chief":
      return append(lines, "chief", "chat", { kind: "text", text: frame.text })
    case "lane":
      return append(lines, frame.avatar, "chat", { kind: "text", text: frame.text })
    case "visible":
      return [...lines, say(frame.author, frame.text)]
    case "part": {
      const part = frame.part
      if (part.kind === "reasoning") return append(lines, frame.author, frame.lane, { kind: "reasoning", text: part.text })
      if (part.kind === "tool_call") return append(lines, frame.author, frame.lane, { kind: "tool", id: part.id, name: part.name, args: part.args })
      return setResult(lines, part)
    }
    default:
      return lines
  }
}
