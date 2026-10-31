/** Length-prefixed JSON frames and the snapshot fold. Wire shapes match `ipc/proto`. */

export const MAX_FRAME_BYTES = 64 * 1024 * 1024

export interface ToolCall {
  id: string
  type?: string
  function: { name: string; arguments: string }
}

export interface Attachment {
  kind?: string
  marker_n: number
  rel_path: string
  mime?: string
  label?: string
}

export interface ChatMessage {
  role: string
  content: string
  tool_calls?: ToolCall[]
  tool_call_id?: string
  attachments?: Attachment[]
  reasoning?: string
}

export interface PlanTodo {
  content: string
  status: string
  locked?: boolean
}

export interface SessionSnap {
  id: string
  name: string
  cwd: string
  messages: ChatMessage[]
  streaming: string | null
  stream_reasoning: string
  working: boolean
  waiting: boolean
  awaiting_approval: boolean
  approval_reason: string | null
  pending_tool_calls: ToolCall[]
  tool_idx: number
  resolved_model_id: string
  effort: string
  pending_steer: string[]
  pending_steer_attachments: Attachment[][]
  subagents: Record<string, unknown>[]
  pending_subagents: Record<string, unknown>[]
  plan_todos: PlanTodo[]
  tokens_in: number
  tokens_out: number
  tokens_cached: number
  cost: number
  context_usage: ContextUsage | null
  sdlc_phase: string | null
  sdlc_branch: string | null
}

export interface ContextUsage {
  prompt_tokens: number
  effective_window: number
  estimated: boolean
  cached_tokens: number | null
  drss_active: boolean
}

export interface GlobalSnap {
  input: string
  cursor: number
  status: string
  palette: string
  accent: string
  mode: unknown
  toast: [string, string] | null
  toast_event_id: string | null
  file_palette: string[] | null
  command_palette: { name: string; desc: string }[]
  palette_sel: number
  agent_mode: string
  pending_attachments: Attachment[]
  work_elapsed_ms: number | null
  follow: boolean
  latest_version: string | null
  current_version: string
  pending_steer_sel: number
  pending_steer_focus: boolean
  subagents_open: boolean
  subagent_sel: number
  agent_viewer: number | null
}

export interface Snapshot {
  foreground_id: string | null
  sessions: SessionSnap[]
  global: GlobalSnap
}

export type RemoteHandoff = {
  target: string
  key: string | null
  newSession: boolean
  sessionId: string | null
  hostId: string | null
}

export type Control =
  | { kind: "exit" }
  | { kind: "disconnect"; text: string }
  | { kind: "frame" }
  | { kind: "home" }
  | { kind: "new"; kill: boolean }
  | { kind: "open"; id: string }
  | { kind: "select" }
  | { kind: "remote"; remote: RemoteHandoff }
  | { kind: "error"; text: string }

export function modeName(mode: unknown): string {
  if (typeof mode === "string") return mode
  if (mode && typeof mode === "object" && !Array.isArray(mode)) {
    const key = Object.keys(mode)[0]
    if (key) return key
  }
  return "unknown"
}

export function foregroundSession(snap: Snapshot): SessionSnap | null {
  const id = snap.foreground_id
  if (id) {
    const found = snap.sessions.find((session) => session.id === id)
    if (found) return found
  }
  return snap.sessions[0] ?? null
}

/** What a daemon event means besides a state fold. */
export function controlOf(event: unknown): Control {
  if (event === "OpenSwapper") return { kind: "home" }
  if (event === "EnterSelect") return { kind: "select" }
  if (!event || typeof event !== "object" || Array.isArray(event)) return { kind: "frame" }
  const record = event as Record<string, unknown>
  if ("Error" in record) return { kind: "error", text: String(record.Error ?? "") }
  if ("NewSession" in record) {
    const body = record.NewSession
    const kill = Boolean(body && typeof body === "object" && (body as { kill?: boolean }).kill)
    return { kind: "new", kill }
  }
  if ("ConnectRemote" in record) {
    const body = asRecord(record.ConnectRemote)
    const target = String(body.target ?? "")
    if (!target) return { kind: "frame" }
    return {
      kind: "remote",
      remote: {
        target,
        key: body.key == null ? null : String(body.key),
        newSession: Boolean(body.new_session),
        sessionId: body.session_id == null ? null : String(body.session_id),
        hostId: body.host_id == null ? null : String(body.host_id),
      },
    }
  }
  if ("AttachSession" in record) {
    const body = record.AttachSession
    const id =
      body && typeof body === "object" ? String((body as { session_id?: string }).session_id ?? "") : ""
    if (id) return { kind: "open", id }
  }
  return { kind: "frame" }
}

export function isSnapshotEvent(event: unknown): boolean {
  return Boolean(event && typeof event === "object" && "Snapshot" in (event as object))
}

export class FrameReader {
  private header = new Uint8Array(4)
  private headerLength = 0
  private body: Uint8Array | null = null
  private bodyLength = 0
  private decoder = new TextDecoder()

  push(chunk: Uint8Array): unknown[] {
    const out: unknown[] = []
    let offset = 0
    while (offset < chunk.length) {
      if (this.body === null) {
        const count = Math.min(4 - this.headerLength, chunk.length - offset)
        this.header.set(chunk.subarray(offset, offset + count), this.headerLength)
        this.headerLength += count
        offset += count
        if (this.headerLength < 4) break
        const length = readU32BE(this.header, 0)
        if (length > MAX_FRAME_BYTES) {
          throw new Error(`frame is ${length} bytes, over the ${MAX_FRAME_BYTES} limit`)
        }
        this.body = new Uint8Array(length)
        this.bodyLength = 0
      }
      const count = Math.min(this.body.length - this.bodyLength, chunk.length - offset)
      this.body.set(chunk.subarray(offset, offset + count), this.bodyLength)
      this.bodyLength += count
      offset += count
      if (this.bodyLength < this.body.length) break
      const body = this.body
      this.body = null
      this.headerLength = 0
      out.push(JSON.parse(this.decoder.decode(body)))
    }
    return out
  }
}

export function encodeFrame(value: unknown): Uint8Array {
  const body = new TextEncoder().encode(JSON.stringify(value))
  if (body.length > MAX_FRAME_BYTES) {
    throw new Error(`frame is ${body.length} bytes, over the ${MAX_FRAME_BYTES} limit`)
  }
  const out = new Uint8Array(4 + body.length)
  writeU32BE(out, 0, body.length)
  out.set(body, 4)
  return out
}

export function decodeFrames(bytes: Uint8Array): unknown[] {
  return new FrameReader().push(bytes)
}

/** Fold one `DaemonEvent` onto the shadow. Control-only events leave it unchanged. */
export function fold(snap: Snapshot | null, event: unknown): Snapshot | null {
  if (!event || typeof event !== "object" || Array.isArray(event)) return snap
  const record = event as Record<string, unknown>
  if ("Snapshot" in record) return normalizeSnapshot(record.Snapshot)
  if ("Delta" in record) return applyDelta(snap, record.Delta)
  return snap
}

export function showImagePath(result: string): string | null {
  const prefix = "image resolved: "
  if (!result.startsWith(prefix)) return null
  const line = result.slice(prefix.length).split("\n")[0] ?? ""
  const cut = line.lastIndexOf(" (")
  if (cut <= 0) return null
  const path = line.slice(0, cut).trim()
  return path.length > 0 ? path : null
}

function applyDelta(snap: Snapshot | null, delta: unknown): Snapshot | null {
  if (!snap || !delta || typeof delta !== "object") return snap
  const record = delta as Record<string, unknown>
  if ("InputChanged" in record) {
    const body = asRecord(record.InputChanged)
    return {
      ...snap,
      global: {
        ...snap.global,
        input: String(body.text ?? ""),
        cursor: numberOr(body.cursor, 0),
      },
    }
  }
  if ("TokenAppended" in record) {
    const body = asRecord(record.TokenAppended)
    const id = String(body.session_id ?? "")
    const text = String(body.text ?? "")
    return mapSession(snap, id, (session) => ({
      ...session,
      streaming: (session.streaming ?? "") + text,
    }))
  }
  if ("ReasoningAppended" in record) {
    const body = asRecord(record.ReasoningAppended)
    const id = String(body.session_id ?? "")
    const text = String(body.text ?? "")
    return mapSession(snap, id, (session) => ({
      ...session,
      stream_reasoning: session.stream_reasoning + text,
    }))
  }
  if ("StatusChanged" in record) {
    const body = asRecord(record.StatusChanged)
    if (body.session_id != null) return snap
    return { ...snap, global: { ...snap.global, status: String(body.text ?? "") } }
  }
  if ("SessionStatusChanged" in record) {
    const body = asRecord(record.SessionStatusChanged)
    const id = String(body.session_id ?? "")
    const working = Boolean(body.working)
    return mapSession(snap, id, (session) => ({
      ...session,
      working,
      waiting: working,
    }))
  }
  if ("ForegroundChanged" in record) {
    const body = asRecord(record.ForegroundChanged)
    return { ...snap, foreground_id: String(body.session_id ?? "") }
  }
  if ("Toast" in record) {
    const body = asRecord(record.Toast)
    const kind = String(body.kind ?? "info")
    const text = String(body.text ?? "")
    return {
      ...snap,
      global: {
        ...snap.global,
        toast: [kind, text],
        toast_event_id: body.id == null ? snap.global.toast_event_id : String(body.id),
      },
    }
  }
  if ("SessionAdded" in record) {
    const session = normalizeSession(record.SessionAdded)
    if (snap.sessions.some((row) => row.id === session.id)) return snap
    return { ...snap, sessions: [...snap.sessions, session] }
  }
  return snap
}

function mapSession(snap: Snapshot, id: string, edit: (session: SessionSnap) => SessionSnap): Snapshot {
  let changed = false
  const sessions = snap.sessions.map((session) => {
    if (session.id !== id) return session
    changed = true
    return edit(session)
  })
  return changed ? { ...snap, sessions } : snap
}

function commandRows(raw: unknown): { name: string; desc: string }[] {
  if (!Array.isArray(raw)) return []
  const rows: { name: string; desc: string }[] = []
  for (const row of raw) {
    if (Array.isArray(row) && row.length >= 2) {
      rows.push({ name: String(row[0]), desc: String(row[1]) })
    } else if (row && typeof row === "object" && "name" in row) {
      const body = row as { name?: unknown; desc?: unknown }
      rows.push({ name: String(body.name ?? ""), desc: String(body.desc ?? "") })
    }
  }
  return rows
}

function normalizeSnapshot(raw: unknown): Snapshot {
  const body = asRecord(raw)
  const global = asRecord(body.global)
  const sessions = Array.isArray(body.sessions) ? body.sessions.map((row) => normalizeSession(row)) : []
  const toast = Array.isArray(global.toast) && global.toast.length >= 2
    ? [String(global.toast[0]), String(global.toast[1])] as [string, string]
    : null
  return {
    foreground_id: body.foreground_id == null ? null : String(body.foreground_id),
    sessions,
    global: {
      input: String(global.input ?? ""),
      cursor: numberOr(global.cursor, 0),
      status: String(global.status ?? ""),
      palette: String(global.palette ?? "dark"),
      accent: String(global.accent ?? "green"),
      mode: global.mode ?? "Chat",
      toast,
      toast_event_id: global.toast_event_id == null ? null : String(global.toast_event_id),
      file_palette: Array.isArray(global.file_palette) ? global.file_palette.map((row) => String(row)) : null,
      command_palette: commandRows(global.command_palette),
      palette_sel: numberOr(global.palette_sel, 0),
      agent_mode: String(global.agent_mode ?? ""),
      pending_attachments: Array.isArray(global.pending_attachments)
        ? (global.pending_attachments as Attachment[])
        : [],
      work_elapsed_ms: global.work_elapsed_ms == null ? null : numberOr(global.work_elapsed_ms, 0),
      follow: global.follow !== false,
      current_version: typeof global.current_version === "string" ? global.current_version : process.env.KOMA_VERSION ?? "",
      latest_version: global.latest_version == null ? null : String(global.latest_version),
      pending_steer_sel: numberOr(global.pending_steer_sel, 0),
      pending_steer_focus: Boolean(global.pending_steer_focus),
      subagents_open: Boolean(global.subagents_open),
      subagent_sel: numberOr(global.subagent_sel, 0),
      agent_viewer: global.agent_viewer == null ? null : numberOr(global.agent_viewer, 0),
    },
  }
}

function normalizeSession(raw: unknown): SessionSnap {
  const body = asRecord(raw)
  return {
    id: String(body.id ?? ""),
    name: String(body.name ?? ""),
    cwd: String(body.cwd ?? ""),
    messages: Array.isArray(body.messages) ? (body.messages as ChatMessage[]) : [],
    streaming: body.streaming == null ? null : String(body.streaming),
    stream_reasoning: String(body.stream_reasoning ?? ""),
    working: Boolean(body.working),
    waiting: Boolean(body.waiting),
    awaiting_approval: Boolean(body.awaiting_approval),
    approval_reason: body.approval_reason == null ? null : String(body.approval_reason),
    pending_tool_calls: Array.isArray(body.pending_tool_calls) ? (body.pending_tool_calls as ToolCall[]) : [],
    tool_idx: numberOr(body.tool_idx, 0),
    resolved_model_id: String(body.resolved_model_id ?? ""),
    effort: String(body.effort ?? ""),
    pending_steer_attachments: Array.isArray(body.pending_steer_attachments) ? body.pending_steer_attachments.map((items) => Array.isArray(items) ? items as Attachment[] : []) : [],
    pending_steer: Array.isArray(body.pending_steer) ? body.pending_steer.map((row) => String(row)) : [],
    subagents: Array.isArray(body.subagents) ? body.subagents.filter((row) => row && typeof row === "object") as Record<string, unknown>[] : [],
    pending_subagents: Array.isArray(body.pending_subagents)
      ? body.pending_subagents.filter((row) => row && typeof row === "object") as Record<string, unknown>[]
      : [],
    plan_todos: Array.isArray(body.plan_todos) ? (body.plan_todos as PlanTodo[]) : [],
    tokens_in: numberOr(body.tokens_in, 0),
    tokens_out: numberOr(body.tokens_out, 0),
    tokens_cached: numberOr(body.tokens_cached, 0),
    cost: numberOr(body.cost, 0),
    context_usage: normalizeUsage(body.context_usage),
    sdlc_phase: body.sdlc_phase == null ? null : String(body.sdlc_phase),
    sdlc_branch: body.sdlc_branch == null ? null : String(body.sdlc_branch),
  }
}

function normalizeUsage(raw: unknown): ContextUsage | null {
  if (!raw || typeof raw !== "object") return null
  const body = raw as Record<string, unknown>
  return {
    prompt_tokens: numberOr(body.prompt_tokens, 0),
    effective_window: numberOr(body.effective_window, 0),
    estimated: Boolean(body.estimated),
    cached_tokens: body.cached_tokens == null ? null : numberOr(body.cached_tokens, 0),
    drss_active: Boolean(body.drss_active),
  }
}

/** Restart when Hello names a different build. A missing Hello is not a mismatch. */
export function needsRestart(hello: string | null, mine: string | undefined, alreadyRestarted: boolean): boolean {
  return Boolean(mine && hello && hello !== mine && !alreadyRestarted)
}

function asRecord(value: unknown): Record<string, unknown> {
  if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>
  return {}
}

function numberOr(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback
}

function readU32BE(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset] ?? 0) << 24) |
    ((bytes[offset + 1] ?? 0) << 16) |
    ((bytes[offset + 2] ?? 0) << 8) |
    (bytes[offset + 3] ?? 0)
  ) >>> 0
}

function writeU32BE(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = (value >>> 24) & 0xff
  bytes[offset + 1] = (value >>> 16) & 0xff
  bytes[offset + 2] = (value >>> 8) & 0xff
  bytes[offset + 3] = value & 0xff
}
