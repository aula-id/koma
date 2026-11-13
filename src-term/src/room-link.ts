import { createConnection, type Socket } from "node:net"
import { encodeFrame, FrameReader } from "./protocol.ts"
import { socketPath } from "./paths.ts"

/** Who a line is for: the user's own conversation, or the team at work. Older saved lines carry none. */
export type Audience = "user" | "team"
const audienceOf = (value: unknown): Audience | undefined => value === "user" || value === "team" ? value : undefined

export type RoomFrame =
  | { type: "hello"; kind: string }
  | { type: "chief"; text: string }
  | { type: "lane"; avatar: string; text: string }
  | { type: "visible"; author: string; text: string; audience?: Audience }
  | { type: "part"; author: string; lane: string; part: LanePart; audience?: Audience }
  | { type: "status"; digest: string; timeline: string; shelf: string; trajectory: string; open: number; verified: number; returned: number; reports: number; lanes: LaneInfo[]; events: EventInfo[]; now_ms: number }
  | { type: "models"; choices: ModelChoice[]; lanes: LaneRow[] }
  | { type: "notice"; text: string }
  | { type: "pasted"; insert: string }
  | { type: "browse"; shelf: ShelfRow[]; goals: Goal[]; reports: Report[] }
  | { type: "item"; key: string; revision: number; updated: number; content: string }
  | { type: "closed" }

/** One row of the shelf tree. */
export type ShelfRow = { key: string; revision: number; updated: number; kind: string; title: string; status: string }
export type GoalNode = { id: string; label: string; kind: string; status: string; severity: string; note: string; resource_key: string; rank: number; row: number }
export type GoalEdge = { from: string; to: string; rel: string }
/**
 * Several trajectories can carry the same goal name. The screens find a goal by that name, so two
 * of them would highlight together and Enter would open the first. Later ones get ` (2)`, ` (3)`.
 */
export function uniqueGoals(goals: Goal[]): Goal[] {
  const seen = new Map<string, number>()
  return goals.map((g) => {
    const n = (seen.get(g.goal_key) ?? 0) + 1
    seen.set(g.goal_key, n)
    return n === 1 ? g : { ...g, goal_key: `${g.goal_key} (${n})` }
  })
}

export type Goal = { goal_key: string; open: number; verified: number; returned: number; nodes: GoalNode[]; edges: GoalEdge[] }
export type Report = { key: string; revision: number; updated: number; title: string; audience: string; severity_summary: string; go_no_go: string; finding_keys: string[]; markdown: string }

/** One structured piece of a lane's turn: thinking, a tool call, a tool result. */
export type LanePart =
  | { kind: "reasoning"; text: string }
  | { kind: "text_delta"; text: string }
  | { kind: "reasoning_delta"; text: string }
  | { kind: "tool_call"; id: string; name: string; args: string }
  | { kind: "tool_result"; id: string; ok: boolean; result: string }

/** One lane in the status table. */
export type LaneInfo = {
  avatar: string
  display: string
  /** A short tag, such as LR for Lab Red. */
  alias: string
  role: string
  /** `chat`, `work`, or empty for an avatar with no lane running. */
  lane: string
  /** `running`, `idle`, `paused`, `submitted`, `done` or `failed`. */
  phase: string
  task: string
  started_ms: number
  last_event_ms: number
  steps: number
  last_tool: string
  error: string
}
/** One milestone on the timeline. */
export type EventInfo = { at_ms: number; avatar: string; marker: string; label: string }

/** A model registered in the coding agent. */
export type ModelChoice = { id: string; name: string; model_id: string; provider: string }
/** What one lane of one avatar runs. */
export type LaneRow = { avatar: string; display: string; lane: string; model: string }

/** One room wire value, or null when it is not a room frame. */
export function parseRoomFrame(value: unknown): RoomFrame | null {
  if (value === "hello") return { type: "hello", kind: "" }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  const body = (key: string) => {
    const found = record[key]
    return found && typeof found === "object" ? found as Record<string, unknown> : null
  }
  const hello = body("hello")
  if (hello) return { type: "hello", kind: String(hello.kind ?? "") }
  const chief = body("chief")
  if (chief) return { type: "chief", text: String(chief.text ?? "") }
  const lane = body("lane_chat")
  if (lane) return { type: "lane", avatar: String(lane.avatar ?? ""), text: String(lane.text ?? "") }
  const visible = body("visible")
  if (visible) return { type: "visible", author: String(visible.author ?? ""), text: String(visible.text ?? ""), audience: audienceOf(visible.audience) }
  const part = body("part")
  if (part) {
    const inner = part.part && typeof part.part === "object" ? part.part as Record<string, unknown> : undefined
    const kind = String(inner?.kind ?? "")
    const author = String(part.author ?? "")
    const lane = String(part.lane ?? "chat")
    const audience = audienceOf(part.audience)
    if (kind === "text_delta" || kind === "reasoning_delta") return { type: "part", author, lane, part: { kind, text: String(inner?.text ?? "") }, audience }
    if (kind === "reasoning") return { type: "part", author, lane, part: { kind, text: String(inner?.text ?? "") }, audience }
    if (kind === "tool_call") return { type: "part", author, lane, part: { kind, id: String(inner?.id ?? ""), name: String(inner?.name ?? ""), args: String(inner?.args ?? "") }, audience }
    if (kind === "tool_result") return { type: "part", author, lane, part: { kind, id: String(inner?.id ?? ""), ok: Boolean(inner?.ok), result: String(inner?.result ?? "") }, audience }
    return null
  }
  const status = body("status")
  if (status) {
    const rows = (value: unknown) => Array.isArray(value) ? value as Record<string, unknown>[] : []
    return {
      type: "status",
      digest: String(status.digest ?? ""),
      timeline: String(status.timeline ?? ""),
      shelf: String(status.shelf ?? ""),
      trajectory: String(status.trajectory ?? ""),
      open: Number(status.open ?? 0),
      verified: Number(status.verified ?? 0),
      returned: Number(status.returned ?? 0),
      reports: Number(status.reports ?? 0),
      lanes: rows(status.lanes).map((l) => ({
        avatar: String(l.avatar ?? ""), display: String(l.display ?? ""), alias: String(l.alias ?? ""), role: String(l.role ?? ""), lane: String(l.lane ?? ""),
        phase: String(l.phase ?? "idle"), task: String(l.task ?? ""), started_ms: Number(l.started_ms ?? 0), last_event_ms: Number(l.last_event_ms ?? 0),
        steps: Number(l.steps ?? 0), last_tool: String(l.last_tool ?? ""), error: String(l.error ?? ""),
      })),
      events: rows(status.events).map((e) => ({ at_ms: Number(e.at_ms ?? 0), avatar: String(e.avatar ?? ""), marker: String(e.marker ?? ""), label: String(e.label ?? "") })),
      now_ms: Number(status.now_ms ?? 0),
    }
  }
  const models = body("models")
  if (models) {
    const rows = (key: string) => Array.isArray(models[key]) ? models[key] as Record<string, unknown>[] : []
    return {
      type: "models",
      choices: rows("choices").map((c) => ({ id: String(c.id ?? ""), name: String(c.name ?? ""), model_id: String(c.model_id ?? ""), provider: String(c.provider ?? "") })),
      lanes: rows("lanes").map((l) => ({ avatar: String(l.avatar ?? ""), display: String(l.display ?? ""), lane: String(l.lane ?? ""), model: String(l.model ?? "") })),
    }
  }
  const browse = body("browse")
  if (browse) {
    const rows = (key: string) => Array.isArray(browse[key]) ? browse[key] as Record<string, unknown>[] : []
    const text = (r: Record<string, unknown>, k: string) => String(r[k] ?? "")
    const num = (r: Record<string, unknown>, k: string) => Number(r[k] ?? 0)
    return {
      type: "browse",
      shelf: rows("shelf").map((r) => ({ key: text(r, "key"), revision: num(r, "revision"), updated: num(r, "updated"), kind: text(r, "kind"), title: text(r, "title"), status: text(r, "status") })),
      goals: uniqueGoals(rows("goals").map((g) => ({
        goal_key: text(g, "goal_key"), open: num(g, "open"), verified: num(g, "verified"), returned: num(g, "returned"),
        nodes: (Array.isArray(g.nodes) ? g.nodes as Record<string, unknown>[] : []).map((n) => ({ id: text(n, "id"), label: text(n, "label"), kind: text(n, "kind"), status: text(n, "status"), severity: text(n, "severity"), note: text(n, "note"), resource_key: text(n, "resource_key"), rank: num(n, "rank"), row: num(n, "row") })),
        edges: (Array.isArray(g.edges) ? g.edges as Record<string, unknown>[] : []).map((e) => ({ from: text(e, "from"), to: text(e, "to"), rel: text(e, "rel") })),
      }))),
      reports: rows("reports").map((r) => ({
        key: text(r, "key"), revision: num(r, "revision"), updated: num(r, "updated"), title: text(r, "title"), audience: text(r, "audience"),
        severity_summary: text(r, "severity_summary"), go_no_go: text(r, "go_no_go"), markdown: text(r, "markdown"),
        finding_keys: Array.isArray(r.finding_keys) ? (r.finding_keys as unknown[]).map(String) : [],
      })),
    }
  }
  const item = body("item")
  if (item) return { type: "item", key: String(item.key ?? ""), revision: Number(item.revision ?? 0), updated: Number(item.updated ?? 0), content: String(item.content ?? "") }
  const pasted = body("pasted")
  if (pasted) return { type: "pasted", insert: String(pasted.insert ?? "") }
  const notice = body("notice")
  if (notice) return { type: "notice", text: String(notice.text ?? "") }
  if ("closed" in record) return { type: "closed" }
  return null
}

/** Attach client for one secure daemon. */
export class RoomLink {
  private socket: Socket | null = null
  private reader = new FrameReader()
  private closed = false
  /** What was sent before the connection was up. A first message typed while the daemon starts is not lost. */
  private outbox: unknown[] = []
  private connected = false

  constructor(private readonly onFrame: (frame: RoomFrame) => void, private readonly onError: (text: string) => void) {}

  async start(sessionId: string): Promise<void> {
    if (this.closed) return
    const socket = createConnection({ path: socketPath(sessionId) })
    await new Promise<void>((resolve, reject) => {
      socket.once("connect", () => resolve())
      socket.once("error", reject)
    })
    this.attach(socket)
  }

  /** Use a connected socket: greet the daemon, then send what was queued while it was starting. */
  attach(socket: Socket): void {
    if (this.closed) { socket.destroy(); return }
    this.socket = socket
    socket.on("data", (data: Buffer) => this.onData(data))
    socket.on("close", () => { if (!this.closed) this.onError("the secure daemon closed the connection") })
    socket.on("error", (error) => { if (!this.closed) this.onError(error.message) })
    this.connected = true
    socket.write(encodeFrame("hello"))
    const waiting = this.outbox
    this.outbox = []
    for (const request of waiting) this.send(request)
  }

  sendUser(text: string): void {
    this.send({ user: { text } })
  }

  /** Text pasted into the composer. The daemon answers with what to insert. */
  paste(text: string): void {
    this.send({ paste: { text } })
  }

  /** Ctrl+V: the daemon reads an image off the clipboard. */
  pasteImage(): void {
    this.send("paste_image")
  }

  /** Ask for the shelf, goals and reports. */
  browse(): void {
    this.send("browse")
  }

  /** Ask for one shelf item in full. */
  open(key: string): void {
    this.send({ open: { key } })
  }

  /** Esc: stop what the Chief and the specialists are saying. Their work keeps running. */
  interrupt(): void {
    this.send("interrupt")
  }

  /** `/stop red` cuts off that specialist's lanes; `all` cuts off every lane. */
  stop(avatar: string): void {
    this.send({ control: { command: { stop: { avatar } } } })
  }

  requestModels(): void {
    this.send("models")
  }

  /** An empty `model` returns the lane to the session default. */
  setModel(avatar: string, lane: string, model: string): void {
    this.send({ set_model: { avatar, lane, model } })
  }

  close(): void {
    this.closed = true
    this.socket?.end()
    this.socket = null
  }

  private send(request: unknown): void {
    if (this.closed) return
    if (!this.connected || !this.socket) {
      this.outbox.push(request)
      return
    }
    this.socket.write(encodeFrame(request))
  }

  private onData(data: Uint8Array): void {
    let frames: unknown[]
    try {
      frames = this.reader.push(data)
    } catch (error) {
      this.onError(error instanceof Error ? error.message : String(error))
      return
    }
    for (const frame of frames) {
      const parsed = parseRoomFrame(frame)
      if (parsed) this.onFrame(parsed)
    }
  }
}

