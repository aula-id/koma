import type { LanePart, RoomFrame } from "./room-link.ts"

/** One piece of a message, in the order the lane produced it. */
export type Item =
  | { kind: "text"; text: string; streaming?: boolean }
  | { kind: "reasoning"; text: string; streaming?: boolean }
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
    if (item.kind === "text" && !item.streaming && tail?.kind === "text" && !tail.streaming) items[items.length - 1] = { kind: "text", text: `${tail.text}\n\n${item.text}` }
    else items.push(item)
    return [...lines.slice(0, -1), { ...last, items }]
  }
  return [...lines, { id: crypto.randomUUID(), author, lane, items: [item] }]
}

/** The line a lane's next piece belongs to: its last message, when that is the same author and lane. */
function currentLine(lines: Line[], author: string, lane: string): number {
  const last = lines[lines.length - 1]
  return last && last.author === author && last.lane === lane && !isYou(last) ? lines.length - 1 : -1
}

/** Add a live piece to the open item of its kind, or start one. */
function stream(lines: Line[], author: string, lane: string, kind: "text" | "reasoning", piece: string): Line[] {
  const at = currentLine(lines, author, lane)
  if (at >= 0) {
    const line = lines[at]!
    const index = line.items.length - 1
    const tail = line.items[index]
    if (tail && tail.kind === kind && tail.streaming) {
      const items = [...line.items]
      items[index] = { kind, text: tail.text + piece, streaming: true }
      return [...lines.slice(0, at), { ...line, items }, ...lines.slice(at + 1)]
    }
  }
  return append(lines, author, lane, { kind, text: piece, streaming: true })
}

/** The whole reply arrived: it replaces the live item of its kind in place, or is added. */
function settle(lines: Line[], author: string, lane: string, kind: "text" | "reasoning", whole: string): Line[] {
  const at = currentLine(lines, author, lane)
  if (at >= 0) {
    const line = lines[at]!
    for (let i = line.items.length - 1; i >= 0; i--) {
      const item = line.items[i]!
      if (item.kind === kind && item.streaming) {
        const items = [...line.items]
        items[i] = { kind, text: whole }
        return [...lines.slice(0, at), { ...line, items }, ...lines.slice(at + 1)]
      }
    }
  }
  return append(lines, author, lane, { kind, text: whole })
}

/** Nothing more is coming for this author: close any open live item. */
function settleAll(lines: Line[], author: string): Line[] {
  return lines.map((line) => line.author === author && line.items.some((item) => "streaming" in item && item.streaming)
    ? { ...line, items: line.items.map((item) => "streaming" in item && item.streaming ? { ...item, streaming: false } : item) }
    : line)
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
      return settle(lines, "chief", "chat", "text", frame.text)
    case "lane":
      return settle(lines, frame.avatar, "chat", "text", frame.text)
    case "visible":
      return [...settleAll(lines, frame.author), say(frame.author, frame.text)]
    case "part": {
      const part = frame.part
      if (part.kind === "text_delta") return stream(lines, frame.author, frame.lane, "text", part.text)
      if (part.kind === "reasoning_delta") return stream(lines, frame.author, frame.lane, "reasoning", part.text)
      if (part.kind === "reasoning") return settle(lines, frame.author, frame.lane, "reasoning", part.text)
      if (part.kind === "tool_call") return append(lines, frame.author, frame.lane, { kind: "tool", id: part.id, name: part.name, args: part.args })
      return setResult(lines, part)
    }
    default:
      return lines
  }
}
