import type { AttachmentEntry } from './session'

export type ToolCallView = {
  id: string
  // Raw tool name, e.g. "bash", "read", "grep", "mcp__foo__bar".
  name: string
  // Raw stringified-JSON arguments object (as the model emitted them).
  args: string
  // Pre-formatted display signature, e.g. `bash(ls src-agent/)`. Optional —
  // derived client-side from name+args when the host doesn't supply it.
  signature?: string
  // Paired Role::Tool result content; null while the call is in flight.
  output: string | null
  // "done" once a matching tool result exists; "pending" otherwise.
  status: 'pending' | 'done'
}

export type ChatMessage = {
  // Stable display index in the projected transcript (host-assigned).
  // Used for React keys, RewindTo, and HistoryPage cursors. Optional so older
  // hosts / synthetic tests still work (FE falls back to array position).
  idx?: number
  role: 'user' | 'assistant'
  // Special render kind for a USER message — the host strips the invisible
  // sentinel and tags it: 'shell' (a `!`-shell `$ cmd`+output entry) or
  // 'bashNudge' (a bg-bash completion nudge). Absent on a plain message.
  kind?: 'shell' | 'bashNudge'
  content: string
  reasoning: string | null
  // Present only on an assistant message that requested tool calls.
  toolCalls?: ToolCallView[]
  // Image attachments on a user message (mirrors the TUI warn attachment card).
  attachments?: AttachmentEntry[]
  computer?: import('../../types/computer').ComputerObservationView
}

// The full palette roles the host pushes (render.rs `PushPalette`,
// `rename_all = "camelCase"`) — the same TUI theme.rs roles `view::draw` uses.
// `bg`/`fg` paint the window chrome; `accent`/`dim`/`panel` drive the chat
// grammar (accent bullets/rails, dim thinking/tool text, the user band = panel).
export type PaletteColors = {
  bg: string
  fg: string
  accent: string
  dim: string
  panel: string
  warn: string
  success: string
  info: string
  error: string
  // Whether this palette reads as a dark theme (host-derived from `bg`'s
  // relative luminance — see `push_rows.rs` `PushPalette::dark`). Optional so
  // an older host build that hasn't started projecting it yet degrades
  // gracefully (consumers fall back to `true`, the dark-default assumption).
  dark?: boolean
}

// One named palette in the host's theme registry, WITH resolved colours (host
// `PushPaletteInfo`) — drives the Settings tab's Appearance grid. `colors` is the
// 11 role colours as `#rrggbb` in the FIXED order [bg, fg, dim, accent, panel,
// sel_bg, sel_fg, success, warn, error, info]. A pick round-trips as SetTheme.
export type PaletteInfo = {
  name: string
  colors: string[]
}

// The Settings tab's Session-section values (host `SettingsValues` reply). `name`/
// `workdir` are session-scoped; the toggles + `internetMode` are per-session prefs;
// `palette` is the active global theme (mirrors config.theme). Null until the first
// GetSettings reply lands.
export type SettingsValues = {
  name: string
  workdir: string[]
  shortSend: boolean
  slidingCache: boolean
  bashSaving: boolean
  codingAutosave: boolean
  internetMode: string
  palette: string
  // The foreground session's stored `/effort` value ("" = model default), for
  // the composer EffortPicker's trigger-pill label.
  effort: string
  // Max agentic turns per sub-agent (≥ 1, default 500).
  subagentMaxTurns: number
  // Legacy wire compatibility only; ignored by deterministic DRSS.
  shortSendEngageN: number
  // Legacy wire compatibility only; ignored by deterministic DRSS.
  shortSendTailN: number
  // Requested reply limit (0 = 128k, bounded by model/context limits).
  maxOutputTokens: number
  contextWindowLimit: number
  contextModelAlias: string
}

// The composer EffortPicker's latest GetEffortOptions reply (host
// `DaemonEvent::EffortOptions`, mirrors the TUI `/effort` menu derivation).
// `state` is "loading" (a catalogue fetch was just armed or is already in
// flight — `options` empty), "unsupported" (the model has no reasoning
// control, or there's no active session — `options` empty), or "ready"
// (`options`/`selected` populated). `note` carries the human-readable
// reason/hint in every state. `null` until the first reply lands (the picker
// shows a loading row); REPLACED wholesale on each reply.
export type EffortOptions = {
  options: string[]
  selected: number
  note: string
  state: 'loading' | 'unsupported' | 'ready'
}

// One day's cost in a UsagePreview's 7-entry daily series (host `PushUsageDay`).
// `epoch` is the LOCAL-midnight unix-seconds boundary for that day.
export type UsageDayEntry = {
  epoch: number
  cost: number
}

// One model row in a UsagePreview's top-3 list (host `PushUsageModel`).
export type UsageModelEntry = {
  modelId: string
  cost: number
  calls: number
}

// The activity-bar Usage panel's LAST-7-DAYS preview (host `UsagePreview` reply),
// straight off the global `~/.koma/usage.sqlite` ledger — host-only, never touches
// the daemon (mirrors FileDiff). `days` is always exactly 7 entries, oldest first.
// Null until the first reply lands (re-requested every time the panel is shown).
export type UsagePreview = {
  cost: number
  tokensIn: number
  tokensCached: number
  tokensOut: number
  calls: number
  days: UsageDayEntry[]
  topModels: UsageModelEntry[]
}

// Analytics tab filter tokens (host `Analytics` request / reply).
