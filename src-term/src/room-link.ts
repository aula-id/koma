import { createConnection, type Socket } from "node:net"
import { encodeFrame, FrameReader } from "./protocol.ts"
import { socketPath } from "./paths.ts"

export type RoomFrame =
  | { type: "hello"; kind: string }
  | { type: "chief"; text: string }
  | { type: "lane"; avatar: string; text: string }
  | { type: "visible"; author: string; text: string }
  | { type: "part"; author: string; lane: string; part: LanePart }
  | { type: "status"; digest: string; timeline: string; shelf: string; trajectory: string; open: number; verified: number; returned: number; reports: number }
  | { type: "models"; choices: ModelChoice[]; lanes: LaneRow[] }
  | { type: "notice"; text: string }
  | { type: "closed" }

/** One structured piece of a lane's turn: thinking, a tool call, a tool result. */
export type LanePart =
  | { kind: "reasoning"; text: string }
  | { kind: "tool_call"; id: string; name: string; args: string }
  | { kind: "tool_result"; id: string; ok: boolean; result: string }

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
  if (visible) return { type: "visible", author: String(visible.author ?? ""), text: String(visible.text ?? "") }
  const part = body("part")
  if (part) {
    const inner = part.part && typeof part.part === "object" ? part.part as Record<string, unknown> : undefined
    const kind = String(inner?.kind ?? "")
    const author = String(part.author ?? "")
    const lane = String(part.lane ?? "chat")
    if (kind === "reasoning") return { type: "part", author, lane, part: { kind, text: String(inner?.text ?? "") } }
    if (kind === "tool_call") return { type: "part", author, lane, part: { kind, id: String(inner?.id ?? ""), name: String(inner?.name ?? ""), args: String(inner?.args ?? "") } }
    if (kind === "tool_result") return { type: "part", author, lane, part: { kind, id: String(inner?.id ?? ""), ok: Boolean(inner?.ok), result: String(inner?.result ?? "") } }
    return null
  }
  const status = body("status")
  if (status) {
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

  constructor(private readonly onFrame: (frame: RoomFrame) => void, private readonly onError: (text: string) => void) {}

  async start(sessionId: string): Promise<void> {
    if (this.closed) return
    const socket = createConnection({ path: socketPath(sessionId) })
    this.socket = socket
    await new Promise<void>((resolve, reject) => {
      socket.once("connect", () => resolve())
      socket.once("error", reject)
    })
    if (this.closed) { socket.destroy(); return }
    socket.on("data", (data: Buffer) => this.onData(data))
    socket.on("close", () => { if (!this.closed) this.onError("the secure daemon closed the connection") })
    socket.on("error", (error) => { if (!this.closed) this.onError(error.message) })
    this.send("hello")
  }

  sendUser(text: string): void {
    this.send({ user: { text } })
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
    if (!this.socket || this.closed) return
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

