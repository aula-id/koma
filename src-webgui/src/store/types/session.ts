export type ModelRoute = {
  role: string
  origin: string
  assignment_uuid: string | null
  source_uuid: string | null
  configured_model: string | null
  provider_uuid: string | null
  effective_model: string | null
  reason: string | null
}

export type HubCookingEntry = {
  kind: 'new' | 'session'
  id: string | null
  name: string
  working?: boolean
  foreground?: boolean
  dirLabel?: string
  currentDir?: boolean
}

export type HubHistoryEntry = {
  id: string
  name: string
  lastActive: number
  dirLabel: string
  currentDir: boolean
}

// A "dying" mark on a session id — set right after firing KillSession
// ('kill', from a COOKING row) or DeleteSession ('delete', from a HISTORY
// row). Kind-scoped (not just the bare id) because a killed session MIGRATES
// from cooking to history on the next Hub push: the same id then briefly
// exists in history too, and an id-only mark would keep disabling that
// migrated-in history row forever (the prune never sees it drop out of
// BOTH lists). A 'kill' mark only ever describes a cooking-row; a 'delete'
// mark only ever describes a history-row.
export type DyingMark = { id: string; kind: 'kill' | 'delete' }

// Whether `id`'s ROW-KIND (`'session'` = cooking row, `'history'` = history
// row) currently carries a matching dying mark. Kind-scoped per `DyingMark` —
// a leftover 'kill' mark from the just-killed session never disables the row
// it migrated INTO (history), and vice versa.
export function isDying(dyingSessions: DyingMark[], id: string, rowKind: 'session' | 'history'): boolean {
  const markKind = rowKind === 'session' ? 'kill' : 'delete'
  return dyingSessions.some((d) => d.id === id && d.kind === markKind)
}

export type SubAgentEntry = {
  // Host-projected subagent id — the kill target for GuiReq KillSubagent.
  // Optional-tolerant: a host build that hasn't started projecting the id yet
  // simply omits it, and the row renders without a kill button. Wire value is
  // a JSON number (render.rs `PushSubAgent.id: usize`), not a string.
  id?: number
  name: string
  status: 'running' | 'done' | 'killed' | 'error'
  summary: string
  // Whether this subagent is already backgrounded (detached). Optional-tolerant like
  // `id`: a host build that hasn't started projecting it omits it, treated as `false`
  // (foreground) so older hosts keep rendering exactly as before.
  detached?: boolean
  // Whether this subagent is currently parking the main turn (has a live tool_call_id).
  // Only `status === 'running' && !detached && blocking` is eligible for the
  // background button / Ctrl+B — mirrors the TUI's `Action::BackgroundSubagent` gate.
  blocking?: boolean
  // ---- Stream-tab content (host `PushSubAgent`, `rename_all = "camelCase"`) ----
  // Present ONLY on the sub-agent the client is streaming into an Explore stream tab
  // (GuiReq SetStreamView); undefined for every other row. `transcript` is the
  // display-ready line log (same source the TUI $-panel renders); `liveText` is the
  // in-progress report tail (dim); `thinking` is the latest reasoning block. A defined
  // `transcript` (even []) means "viewed"; undefined means "not viewed yet / loading".
  transcript?: string[]
  liveText?: string
  thinking?: string
}

export type BashJobEntry = {
  id: string
  cmd: string
  status: 'running' | 'done' | 'killed' | 'error'
  // The captured output tail (host `PushBashJob.outputTail`), present ONLY on the job the
  // client is streaming into a stream tab; undefined for every other row. A defined value
  // (even '') means "viewed"; undefined means "not viewed yet / loading".
  outputTail?: string
}

// One cumulative file-change row for the Explore "File changed" panel — the
// (workspace-relative when possible) path this session's write/edit/delete
// touched + its latest status. Persisted daemon-side (survives compaction +
// close/reopen), REPLACED wholesale on each Snapshot.
export type FileChangeEntry = {
  path: string
  status: 'added' | 'modified' | 'deleted'
}

// One Plan-mode todo row for the Explore "PLAN" section — mirrors the host's
// `PlanTodoSnapshot` (render.rs `PushPlanTodo`, `rename_all = "camelCase"`).
// The two locked workflow rails ("serve plan to user"/"save plan to file &
// prompt approval") ride this too now, flagged via `locked` (TUI parity: the
// rails show right after `plan_enter`, before the model's first `checklist`).
// Empty array = not in Plan mode, or no plan yet.
export type PlanTodoEntry = {
  content: string
  status: 'pending' | 'in_progress' | 'completed' | 'cancelled'
  locked: boolean
  /** SDLC graph node id when projected from mission graph. */
  nodeId?: string
}

// Plan-todo rows that count toward the visible checklist — the locked
// workflow rails are internal bookkeeping and excluded from any done/total
// count (both the Explore PLAN section header and the UsageFooter badge
// share this so they can never disagree).
export function visiblePlanTodos(todos: PlanTodoEntry[]): PlanTodoEntry[] {
  return todos.filter((t) => !t.locked)
}

// Mirrors the Rust host's `PushAttachment` (render.rs, `rename_all = "camelCase"`):
// `markerN` (the daemon's `[Image #N]` marker number) round-trips back in
// `RemoveAttachment`; `name` is the on-disk basename; `kind` is the mime-derived
// chip kind. Full array — REPLACED on each Snapshot, never accumulated.
export type AttachmentEntry = {
  markerN: number
  name: string
  kind: 'image' | 'file'
}

export type SearchResultEntry = {
  path: string
  label: string
}

// One phase of the TUI-parity startup splash (host `Loading` push envelope) —
// mirrors the TUI's cold-session warm-up phase lines ("indexing workspace" /
// "reading project docs"). 'pending' = not started yet, 'running' = in
// progress, 'done'/'skipped'/'failed' are terminal.
export type LoadPhase = 'pending' | 'running' | 'done' | 'skipped' | 'failed'

// GUI-only attach hydration. These phases describe work in the native webview
// bridge, not daemon warm-up: each corresponding envelope is applied on a
// separate animation frame so WebKit gets a paint between expensive state swaps.
export type BootstrapState = {
  session: LoadPhase
  config: LoadPhase
  settings: LoadPhase
  repos: LoadPhase
}

// The tool call the session is currently PARKED on awaiting a decision (host
// `pending_tool_calls[tool_idx]` while `awaiting_approval` is set — approval.rs).
// `name`/`args` are the raw tool name + stringified-JSON arguments; `signature`
// is the host's pre-formatted display line when it supplies one. When
// `name == "plan_ready"` this is a PLAN decision (rendered inline in the chat as
// the plan digest + approve/compact/deny controls), otherwise it's a risky/
// classifier-flagged TOOL approval (rendered as the modal approval card).
export type PendingCall = {
  id?: string
  name: string
  args: string
  signature?: string
}

// One transient toast — the host's per-session `SessionRuntime.toast`
// (state/runtime.rs `set_toast`/`set_toast_info`) projected via the Status
// envelope. `id` is a client-minted monotonic tick so a repeat/re-fired toast
// re-triggers the auto-dismiss timer + re-mounts the card even when the text is
// unchanged; `kind` drives which lucide icon + palette role tints it (the
// container itself is always the neutral themed surface — see ToastContainer).
// The wire (render.rs) currently only ever emits "error"/"info"; "warn"/
// "success" are accepted here so the client is ready without a Rust change.
// Safeguard blocks (harness flagged / classifier unavailable) arrive here.
export type ToastEntry = {
  id: number
  text: string
  kind: 'error' | 'warn' | 'success' | 'info'
}

// The host's FileDiff reply payload for a `kind:'diff'` editor tab — the
// original/modified contents of a File-changed path plus its status flags.
// `error` non-null → render the message instead of an editor; `binary` → a
// "binary file" notice; a NEW file → `original === ''` (all-added); a DELETED
// file → `modified === ''` (all-removed).
export type DiffPayload = {
  original: string
  modified: string
  error: string | null
  binary: boolean
  // Where the original side came from: 'git' (git show HEAD:) or 'baseline' (the
  // session's "virtual git" first-touch pre-image — non-git directories). DiffTab
  // shows a dim "session baseline" badge for the latter.
  origin: 'git' | 'baseline'
}

// One agent dashboard entry (host `AgentEntry`, ALWAYS re-pushed wholesale in
// the `AgentsValues` envelope — never accumulated). NOTE the wire's nested
// structs are snake_case (unlike the envelope's own camelCase field names —
// see the `AgentsValues` push comment), so the reducer maps `model_uuid` ->
// `modelUuid` etc.; this is the store-normalized, camelCase shape components
// consume. `source` distinguishes a built-in agent (ships with koma, no
// delete) from a global override (shared across sessions) or a session-local
// one. `modelUuid`/`model` are both null when the agent inherits the
// session's main model — always true for builtins (they never carry a model
// override). `tools` can legitimately be an empty array (falls back to the
// default read-only tool set at USE time daemon-side; an empty array here is
// NOT "no tools allowed").
export type AgentEntry = {
  name: string
  description: string
  conditions: string
  source: 'session' | 'global' | 'builtin' | 'extension'
  modelUuid: string | null
  // Host-resolved display name for `modelUuid` (informational only — the
  // Agents panel/tab re-resolve `modelUuid` against `catalogueModels`/
  // `catalogueProviders` themselves for the "name @ provider" label, per the
  // locked design, rather than trusting this field's exact format).
  model: string | null
  tools: string[]
  prompt: string
  // The owning extension's manifest id, set only when `source === 'extension'`
  // (host `ext_id`, mirrors `AgentDef::ext_id`) — cross-referenced against the
  // extensions slice's `installed` list for a display name; falls back to the
  // raw id when not (yet) resolvable. `null` for every other source.
  extId: string | null
}

export type LspServerStatus = {
  id: string
  name: string
  binary: string
  source: 'managed' | 'path' | 'missing'
  path?: string
  version?: string
  extensions: string[]
  installKind: string
  package: string
}

// Transient install progress for one server id (LspInstall push).
export type LspInstallProgress = {
  id: string
  pct: number
  error: string | null
}

// Live language-server runtime row (LspRuntime push) — footer Language Servers drawer.
export type LspRuntimeServer = {
  id: string
  name: string
  root: string
  /** starting | ready | working | error */
  phase: string
  title?: string | null
  message?: string | null
  percentage?: number | null
  openDocs: number
}

// One entry in the Agents dashboard's model catalogue (host
// `CatalogueModelSnapshot`, snake_case on the wire — see `AgentsValues`).
export type CatalogueModelEntry = { uuid: string; name: string; modelId: string; providerUuid: string }

// One entry in the Agents dashboard's provider catalogue (host
// `CatalogueProviderSnapshot`, snake_case on the wire).
export type CatalogueProviderEntry = { uuid: string; name: string; endpoint: string }

// Resolve a `modelUuid` to its "name @ provider" display label for the Agents
// panel row / AgentTab's dim model line — `null` or an unresolvable uuid (a
// stale/deleted model) both fall back to "(inherit main)".
export function resolveModelLabel(
  modelUuid: string | null,
  catalogueModels: CatalogueModelEntry[],
  catalogueProviders: CatalogueProviderEntry[],
): string {
  if (!modelUuid) return '(inherit main)'
  const m = catalogueModels.find((x) => x.uuid === modelUuid)
  if (!m) return '(inherit main)'
  const p = catalogueProviders.find((x) => x.uuid === m.providerUuid)
  return p ? `${m.name} @ ${p.name}` : m.name
}
