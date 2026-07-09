import { create } from 'zustand'
import type { McpServer, Provider, Model, ModelListEntry, RouteEntry } from '../types/config'

// ---- Bridge contract types (Rust -> JS push envelopes) ----------------

// One tool call folded onto its assistant message, with its paired result.
// Mirrors the host's fuller turn projection (render.rs `PushToolCall`,
// `rename_all = "camelCase"`): the assistant message holds the calls; each
// `Role::Tool` result is joined back by id and inlined as `output`, matching
// how the TUI resolves completion (⚙→✓) + the result box FRESH every frame.
// All fields optional-tolerant: a host build that hasn't started projecting
// the fuller shape yet simply omits `toolCalls`, and the UI degrades to the
// plain message body.
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
  internetMode: string
  palette: string
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

export type SubAgentEntry = {
  // Host-projected subagent id — the kill target for GuiReq KillSubagent.
  // Optional-tolerant: a host build that hasn't started projecting the id yet
  // simply omits it, and the row renders without a kill button. Wire value is
  // a JSON number (render.rs `PushSubAgent.id: usize`), not a string.
  id?: number
  name: string
  status: 'running' | 'done' | 'killed' | 'error'
  summary: string
}

export type BashJobEntry = {
  id: string
  cmd: string
  status: 'running' | 'done' | 'killed' | 'error'
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
// rails show right after `plan_enter`, before the model's first `todowrite`).
// Empty array = not in Plan mode, or no plan yet.
export type PlanTodoEntry = {
  content: string
  status: 'pending' | 'in_progress' | 'completed' | 'cancelled'
  locked: boolean
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

// The tool call the session is currently PARKED on awaiting a decision (host
// `pending_tool_calls[tool_idx]` while `awaiting_approval` is set — approval.rs).
// `name`/`args` are the raw tool name + stringified-JSON arguments; `signature`
// is the host's pre-formatted display line when it supplies one. When
// `name == "plan_ready"` this is a PLAN decision (rendered inline in the chat as
// the plan digest + approve/compact/deny controls), otherwise it's a risky/
// classifier-flagged TOOL approval (rendered as the modal approval card).
export type PendingCall = {
  name: string
  args: string
  signature?: string
}

// One transient toast — the host's per-session `SessionRuntime.toast`
// (state/runtime.rs `set_toast`/`set_toast_info`) projected via the Status
// envelope. `id` is a client-minted monotonic tick so a repeat/re-fired toast
// re-triggers the auto-dismiss timer + re-mounts the card even when the text is
// unchanged; `kind` drives severity colouring (error vs info). Safeguard blocks
// (harness flagged / classifier unavailable) arrive here.
export type ToastEntry = {
  id: number
  text: string
  kind: 'error' | 'info'
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
}

// One editor tab over the main content column. tabs[0] is ALWAYS the permanent,
// uncloseable chat tab; diff tabs are opened from the Explorer's File-changed
// rows. The `kind` discriminant is deliberately left open — a future
// `{ kind: 'session' }` variant (multi-session tabs, deferred but planned) slots
// in additively without disturbing existing consumers.
export type Tab =
  | { id: 'chat'; kind: 'chat' }
  // The singleton Settings page (VSCode-style), opened from the ActivityBar gear.
  // Deduped by the fixed id 'settings'; closeable like a diff tab.
  | { id: 'settings'; kind: 'settings' }
  | {
      // Stable id `diff:${path}`, so find-by-path (open/dedupe) is trivial.
      id: string
      kind: 'diff'
      // The path exactly as the fileChanges record carries it — the key for the
      // FileDiff req + reply.
      path: string
      // Basename of `path`. TabBar adds a dim parent-dir suffix at render time
      // when two open tabs share a basename (collision depends on the live tab
      // set, so it's resolved there, not baked into the stored title).
      title: string
      // Filled by the FileDiff reply; undefined until the first reply lands.
      diff?: DiffPayload
      // True while a FileDiff req is in flight (initial open OR a re-request on
      // re-activate). A stale `diff` keeps rendering while loading so re-focus
      // never flashes to a spinner.
      loading: boolean
    }

export type PushEnvelope =
  | {
      k: 'Snapshot'
      session: string
      state: string
      messages: ChatMessage[]
      title: string
      palette: PaletteColors
      subagents: SubAgentEntry[]
      bash: BashJobEntry[]
      // Cumulative file-change log (#24). Optional-tolerant: a host build that
      // doesn't project it yet omits it, and the panel shows "No changes".
      fileChanges?: FileChangeEntry[]
      // Plan-mode todo checklist (Explore "PLAN" section). Optional-tolerant:
      // a host build that doesn't project it yet leaves the panel's PLAN
      // section empty (as if no plan were in progress).
      planTodos?: PlanTodoEntry[]
      attachments: AttachmentEntry[]
      // Global agent mode token ("auto"/"normal"/"plan"/"yolo"), projected from
      // the host's process-global agent_mode. Optional-tolerant: a host build
      // that doesn't project it yet leaves the store's current mode untouched.
      mode?: string
      // Queued mid-turn steer previews (host `SessionSnapshot.pending_steer`):
      // messages submitted while the turn is cooking, capped at 5 daemon-side.
      // Truncated one-line previews. Optional-tolerant: a host build that doesn't
      // project it yet leaves the store's queue empty.
      pendingSteer?: string[]
      // Approval/plan-decision gate (host `awaiting_approval` — approval.rs).
      // True when the turn is PARKED waiting on a y/a/n decision. The paused
      // call rides along in `pendingCall` (name/args); `approvalReason` is the
      // classifier's `verdict.reason` for a risky pause (null for a plan_ready
      // pause or a non-classifier park). Optional-tolerant: a host build that
      // doesn't project these yet leaves the gate closed.
      awaitingApproval?: boolean
      approvalReason?: string | null
      pendingCall?: PendingCall | null
    }
  // Swap-START signal pushed the instant a Select/New is acted on host-side,
  // BEFORE teardown, so the loader rises deterministically across the
  // uninterruptible attach gap (matches Rust PushEnvelope::Switching { to }).
  // `to` is the target session id/uuid — resolved to a friendly hub label,
  // falling back to any optimistic label already raised, then a generic one.
  | { k: 'Switching'; to: string }
  | { k: 'StreamMsg'; session: string; text: string }
  | { k: 'Reasoning'; session: string; text: string }
  // `toast` is the transient message text (safeguard/harness/classifier notices
  // + generic host toasts). `kind` is the severity token ("error"/"info") the
  // host now carries alongside the text so the GUI can colour error vs info —
  // optional-tolerant: a host build that doesn't project it yet defaults to info.
  // The five usage fields (tokensIn/tokensCached/tokensOut/cost/mode) drive the
  // chat column's UsageFooter statusline; optional-tolerant for an older host
  // build that doesn't project them yet (default 0 / 'auto' in the reducer).
  | {
      k: 'Status'
      session: string
      working: boolean
      toast: string | null
      toastKind?: string
      tokensIn?: number
      tokensCached?: number
      tokensOut?: number
      cost?: number
      mode?: 'auto' | 'normal' | 'plan' | 'yolo'
    }
  | {
      k: 'Hub'
      state: string
      cooking: HubCookingEntry[]
      history: HubHistoryEntry[]
    }
  | { k: 'SearchResults'; query: string; items: SearchResultEntry[] }
  // Authoritative config projection (mcp/providers/models) — global, not
  // per-session. REPLACES the whole config slice, pushed on config change and
  // on (re)attach. Also carries the active palette (theme) — Config is pushed
  // in BOTH the empty/swapper state and the attached state (render.rs
  // `PushEnvelope::Config.palette`), so it's the one push the empty/swapper
  // state — which never emits a Snapshot — can rely on to repaint to
  // config.json's theme instead of falling back to the dark default.
  | {
      k: 'Config'
      mcp: McpServer[]
      providers: Provider[]
      models: Model[]
      palette?: PaletteColors
      // Onboarding gate: the host's authoritative first-run flag (Rust
      // `Mode::Onboard` — no usable Main route). Optional-tolerant: a host
      // build that doesn't project it yet leaves it undefined, and the UI
      // derives first-run from an empty/unconfigured config instead.
      firstRun?: boolean
      // Active theme (palette) name — the currently-selected key in the host's
      // named-palette registry (theme.rs). Drives the onboarding theme picker's
      // active row. Optional-tolerant.
      theme?: string
      // Available theme (palette) names the host advertises (theme.rs
      // registry). The onboarding picker lists these; falls back to a bundled
      // KNOWN_THEMES list when the host omits them.
      themes?: string[]
      // Full palette catalogue WITH resolved colours (host `PushPaletteInfo`),
      // for the Settings tab's Appearance grid. Optional-tolerant: absent on a
      // host build that doesn't project it yet (the grid then falls back to the
      // names-only `themes` list rendered as label chips).
      palettes?: PaletteInfo[]
    }
  // Reply to GuiReq ListModels — live per-provider model-id catalogue. Field
  // is `models` to match the daemon's PushEnvelope::ModelList { provider, models }.
  | { k: 'ModelList'; provider: string; models: ModelListEntry[] }
  // Reply to GuiReq ListRoutes — live per-model OpenRouter endpoint list. Echoes
  // the provider+modelId it was fetched for so ModelForm can discard a stale
  // reply that no longer matches its current selection. Empty `routes` = a
  // non-OpenRouter provider (UI shows only the synthetic "Auto" row).
  | { k: 'RouteList'; provider: string; modelId: string; routes: RouteEntry[] }
  // Reply to GuiReq FileDiff — the original/modified contents of a File-changed
  // path, for a Monaco diff tab. Echoes the `path` it was fetched for (the tab
  // key). A reply is guaranteed for every request; the reducer ignores a reply
  // whose tab was closed meanwhile.
  | {
      k: 'FileDiff'
      path: string
      original: string
      modified: string
      error: string | null
      binary: boolean
    }
  // Reply to GuiReq UsagePreview — a LAST-7-DAYS usage preview computed straight
  // off the global usage ledger (host-only, never touches the daemon). ALWAYS a
  // reply so the Usage panel's loading state can never hang. `scope` echoes the
  // request's "all"/"session" token so a reply that no longer matches the
  // CURRENTLY selected scope (a rapid toggle racing an in-flight request) is
  // dropped rather than rendered.
  | {
      k: 'UsagePreview'
      cost: number
      tokensIn: number
      tokensCached: number
      tokensOut: number
      calls: number
      days: UsageDayEntry[]
      topModels: UsageModelEntry[]
      scope: string
    }
  // Reply to GuiReq GetSettings (and the re-push after SetPrefs) — the Settings
  // tab's Session-section values + active palette. Guaranteed for every request
  // (even detached: the host answers from global config with defaults).
  | {
      k: 'SettingsValues'
      name: string
      workdir: string[]
      shortSend: boolean
      slidingCache: boolean
      bashSaving: boolean
      internetMode: string
      palette: string
    }

// GuiReq (JS -> Rust request payloads) is a global ambient type declared in
// koma.d.ts alongside the rest of the window bridge contract.

// ---- Store shape --------------------------------------------------------

type SessionSlice = {
  id: string | null
  state: string | null
  messages: ChatMessage[]
  title: string
  working: boolean
  stream: string
  reasoning: string
  subagents: SubAgentEntry[]
  bash: BashJobEntry[]
  fileChanges: FileChangeEntry[]
  // Plan-mode todo checklist (Explore "PLAN" section). REPLACED wholesale on
  // each Snapshot; empty outside Plan mode or before a plan exists.
  planTodos: PlanTodoEntry[]
  attachments: AttachmentEntry[]
  searchResults: SearchResultEntry[]
  // Global agent mode token ("auto"/"normal"/"plan"/"yolo"), projected from the
  // host's process-global agent_mode via the Snapshot envelope. Drives the
  // composer mode selector. Defaults to "auto".
  mode: string
  // Queued mid-turn steer previews (host `SessionSnapshot.pending_steer`) —
  // submits made while the turn is cooking are queued daemon-side (cap 5) rather
  // than starting a new turn. Drives the composer's pending-steer indicator +
  // the send cap. REPLACED wholesale on each Snapshot.
  pendingSteer: string[]
  // Approval gate (host `awaiting_approval`): true while the turn is parked on a
  // y/a/n decision. Drives the ApprovalOverlay modal (risky/classifier pause) +
  // the inline plan controls (plan_ready pause). REPLACED on each Snapshot.
  awaitingApproval: boolean
  // The classifier's reason for a risky pause (null for a plan_ready / non-
  // classifier park). Shown as the "why" in the approval card.
  approvalReason: string | null
  // The tool call the session is parked on (name/args of
  // pending_tool_calls[tool_idx]); null when not awaiting. Distinguishes a plan
  // decision (`name === 'plan_ready'`) from a tool approval.
  pendingCall: PendingCall | null
  // Usage counters + running cost projected on every Status push (host
  // token-accounting). Drive the UsageFooter statusline. Default to 0 when the
  // host hasn't projected them yet.
  tokensIn: number
  tokensCached: number
  tokensOut: number
  cost: number
}

type HubSlice = {
  state: string | null
  cooking: HubCookingEntry[]
  history: HubHistoryEntry[]
}

// Global config (not per-session) — authoritative from the daemon's
// AppConfig projection. Always REPLACED wholesale by a Config push, never
// accumulated.
type ConfigSlice = {
  mcp: McpServer[]
  providers: Provider[]
  models: Model[]
  // True once the first authoritative Config push has landed. The pre-session
  // gate (start screen vs onboarding) waits on this so it never flashes
  // onboarding against the empty initial slice before the host reports config.
  loaded: boolean
  // Host's first-run flag (see Config envelope). Undefined until pushed.
  firstRun?: boolean
  // Active theme name + advertised theme registry (see Config envelope).
  theme: string
  themes: string[]
  // Full palette catalogue with resolved colours (Settings Appearance grid).
  // Empty until the first Config push that carries it.
  palettes: PaletteInfo[]
}

// Local-only UI state (never pushed by the host, never sent upstream) — the
// omnisearch overlay's open/closed flag. Kept in the store (rather than
// component state) so the Composer, nested under a different route subtree
// than RootLayout's overlay mount point, can open it without prop drilling.
type UiSlice = {
  omnisearchOpen: boolean
  // One-shot signal: a workspace path picked from OmniSearchPalette, queued
  // for the Composer to append into its local draft text. The daemon's
  // attachment ingest is image-only, so omnisearch picks are inserted as a
  // plain path reference (for the model to read via its own tools) rather
  // than routed through AttachPath. Composer consumes this via useEffect and
  // clears it with consumeComposerInsert so it doesn't re-fire on rerender.
  composerInsert: string | null
  // One-shot signal: text to REPLACE the Composer's draft with, queued by a
  // rewind (the hover-edit pencil on a user bubble). Distinct from
  // `composerInsert` (which APPENDS an omnisearch path) — rewind refills the
  // whole draft with the rewound message's text for editing + resend. Composer
  // consumes it via useEffect and clears it with consumeComposerRefill.
  composerRefill: string | null
  // Staged rewind (edit pencil): the DISPLAY index of the user message being
  // edited, remembered so the NEXT send fires `RewindTo(index)` before `Submit`
  // (rewind-on-send). `null` when no rewind is staged. Set by `stageRewind` (edit
  // click), cleared by `clearRewind` (send commits it, or the composer is emptied
  // to cancel). Clicking edit does NOT truncate — the chat stays visible until send.
  pendingRewindIndex: number | null
  // Monotonic tick bumped on every send. ChatView watches it to FORCE a
  // jump-to-bottom (re-engaging the scroll-stick regardless of scroll position)
  // when the user submits while scrolled up. Not a boolean so repeat sends at
  // the same scroll position still fire the effect.
  scrollTick: number
  // Full-screen session-swap overlay: set optimistically the moment
  // SelectSession/NewSession is emitted from ResumePalette, holding the
  // target session's display name. There is no host-pushed "swap started"
  // signal on this build (the attach can block synchronously for several
  // seconds — build-skew daemon restarts, cold session spawn), so the next
  // authoritative Snapshot is the only reliable clear point. `null` = no
  // swap in flight.
  switchingTo: string | null
  // Active transient toast (host safeguard/harness/generic notice), or null when
  // none is showing. Set from the Status envelope's `toast`/`kind`; cleared by
  // ToastContainer's auto-dismiss (or a newer toast replacing it). Deduped by
  // text so a host that re-pushes the same live toast on every Status tick
  // doesn't keep resetting the timer.
  toast: ToastEntry | null
  // Monotonic counter minting `ToastEntry.id` — guarantees each distinct toast
  // gets a fresh id (and thus a fresh dismiss timer) even after a null gap.
  toastSeq: number
  // VSCode-style editor tabs over the main content column. tabs[0] is always
  // the permanent chat tab; diff tabs append as File-changed rows are opened.
  // Local-only UI state — never pushed by the host. Reset to just the chat tab
  // on a genuine session switch (a diff tab is file-change context for the OLD
  // session).
  tabs: Tab[]
  // The shown tab's id — 'chat' or a `diff:${path}`.
  activeTabId: string
  // Monotonic tick bumped by `focusPlanSection` (the UsageFooter PLAN badge
  // click): a cross-tree signal, mirrors `scrollTick`. RootLayout watches it
  // to open the Explore sidebar/panel; ExplorePanel watches it to expand its
  // PLAN section — both live outside the Composer/footer's subtree, so a
  // store tick (not a prop) is how the click reaches them.
  focusPlanTick: number
  // Usage panel scope toggle ("all" = global last-7-days [default], "session" =
  // the same window filtered to the CURRENT session's ledger rows only). The
  // Sidebar header's all/session control is HIDDEN whenever there's no current
  // session (the welcome/start screen) and this is forced back to "all" the
  // instant a session goes away while "session" was selected (see UsagePanel's
  // session-loss effect + `setUsageScope`).
  usageScope: 'all' | 'session'
}

type KomaState = {
  session: SessionSlice
  hub: HubSlice
  palette: PaletteColors
  ui: UiSlice
  config: ConfigSlice
  // Live per-provider model-id catalogue, keyed by the most recent
  // ListModels reply's provider (see ModelForm's provider-select trigger).
  modelList: ModelListEntry[]
  // Live per-model route (OpenRouter endpoint) list from the most recent
  // ListRoutes reply — carries the provider+modelId it was fetched for so the
  // consumer can ignore a stale reply. `null` until the first reply lands.
  routeList: { provider: string; modelId: string; routes: RouteEntry[] } | null
  // The Settings tab's Session-section values from the latest GetSettings /
  // SetPrefs re-push. `null` until the first reply lands (the tab shows a
  // loading row); REPLACED wholesale on each reply.
  settingsValues: SettingsValues | null
  // The activity-bar Usage panel's latest LAST-7-DAYS preview. `null` until the
  // first reply lands (the panel shows a loading row); REPLACED wholesale on
  // each reply. The panel re-requests it every time it's shown.
  usagePreview: UsagePreview | null
  // Rust -> JS: apply an authoritative push envelope. Always REPLACES the
  // relevant slice fields — never accumulates/appends.
  push: (env: PushEnvelope) => void
  // JS -> Rust: typed request helper, tags the envelope { t: 'req', ...g }.
  req: (g: GuiReq) => void
  openOmniSearch: () => void
  closeOmniSearch: () => void
  // Queue a workspace path for the Composer to insert into its draft text.
  insertToComposer: (path: string) => void
  // Composer-side ack: clears the one-shot signal after consuming it.
  consumeComposerInsert: () => void
  // Queue text to REPLACE the Composer draft (rewind refill). Called right after
  // a RewindTo request so the rewound message drops back into the composer.
  refillComposer: (text: string) => void
  // Composer-side ack: clears the refill one-shot after consuming it.
  consumeComposerRefill: () => void
  // Stage a rewind-on-send: remember the DISPLAY index of the message being edited
  // (the edit pencil). The Composer fires RewindTo(index) then Submit on send.
  stageRewind: (index: number) => void
  // Clear a staged rewind (send committed it, or the user emptied the composer).
  clearRewind: () => void
  // Bump scrollTick to force ChatView to jump to the bottom (on send).
  requestScrollBottom: () => void
  // Optimistically raise the session-swap overlay with the target's display
  // name. Called right before the SelectSession/NewSession request is sent.
  startSwitching: (name: string) => void
  // Best-effort cancel: dismisses the overlay locally. The in-flight swap on
  // the host side cannot be interrupted, so this only stops showing the
  // loader — the eventual Snapshot for the target session still lands and is
  // applied normally.
  cancelSwitching: () => void
  // Dismiss the active toast (auto-dismiss timer, or a manual close). No-op if
  // the id no longer matches the current toast (a newer toast already replaced
  // it — its own timer owns the dismissal).
  dismissToast: (id: number) => void
  // Open (or focus) the singleton Settings tab (id 'settings'): find-or-create,
  // activate it, and fire GetSettings so its values refresh. Mirrors openDiffTab's
  // dedupe + activate shape.
  openSettingsTab: () => void
  // Open (or focus) a Monaco diff tab for a File-changed `path`: find-by-path or
  // create, mark it loading, fire the FileDiff req, and activate it. Re-opening
  // an already-open file refreshes it (same loading + re-request path).
  openDiffTab: (path: string) => void
  // Close a diff tab (never 'chat'). If it was the active tab, activate the
  // adjacent (left) tab — tabs[0] is always the chat tab, so a fallback exists.
  closeTab: (id: string) => void
  // Activate a tab. Re-focusing a diff tab RE-REQUESTS its FileDiff for
  // freshness (contents may have changed since it was opened) while keeping the
  // stale diff on screen so the editor doesn't flash.
  activateTab: (id: string) => void
  // The UsageFooter PLAN badge click (Plan mode only): bump `focusPlanTick` so
  // RootLayout opens the Explore sidebar/panel and ExplorePanel expands its
  // PLAN section in response.
  focusPlanSection: () => void
  // The Sidebar Usage-panel header's all/session segmented control: switch
  // scope. UsagePanel re-requests on the resulting change.
  setUsageScope: (scope: 'all' | 'session') => void
}

const initialSession: SessionSlice = {
  id: null,
  state: null,
  messages: [],
  title: '',
  working: false,
  stream: '',
  reasoning: '',
  subagents: [],
  bash: [],
  fileChanges: [],
  planTodos: [],
  attachments: [],
  searchResults: [],
  mode: 'auto',
  pendingSteer: [],
  awaitingApproval: false,
  approvalReason: null,
  pendingCall: null,
  tokensIn: 0,
  tokensCached: 0,
  tokensOut: 0,
  cost: 0,
}

const initialHub: HubSlice = {
  state: null,
  cooking: [],
  history: [],
}

// The permanent chat tab (id 'chat'), always tabs[0] and never closeable. A
// factory (not a shared const) so every reset gets a fresh array/object.
const makeChatTab = (): Tab => ({ id: 'chat', kind: 'chat' })

const initialUi: UiSlice = {
  omnisearchOpen: false,
  composerInsert: null,
  composerRefill: null,
  pendingRewindIndex: null,
  scrollTick: 0,
  switchingTo: null,
  toast: null,
  toastSeq: 0,
  tabs: [makeChatTab()],
  activeTabId: 'chat',
  focusPlanTick: 0,
  usageScope: 'all',
}

// Bundled fallback theme (palette) registry — mirrors the host's theme.rs
// PALETTES names 1:1. Used as the onboarding picker's list when the host build
// doesn't advertise a `themes` array on the Config push yet.
export const KNOWN_THEMES = [
  'dark',
  'light',
  'forest',
  'autumn',
  'warm',
  'cold symphony',
  'winter',
  'monokai',
  'vscode',
  'github dark',
] as const

const initialConfig: ConfigSlice = {
  mcp: [],
  providers: [],
  models: [],
  loaded: false,
  firstRun: undefined,
  theme: 'dark',
  themes: [...KNOWN_THEMES],
  palettes: [],
}

const initialModelList: ModelListEntry[] = []

const initialRouteList: KomaState['routeList'] = null

const initialPalette: PaletteColors = {
  bg: '#0b0e14',
  fg: '#c8d3f5',
  accent: '#39ff14',
  dim: '#adadad',
  panel: '#2b2f38',
}

const HEX_RE = /^#[0-9a-fA-F]{6}$/

// Live palette sync: repaint the --koma-* CSS vars whenever a Snapshot lands
// with a palette (home of the glue that used to live in Terminal.tsx's OSC 5380
// handler). Sets the full role set — bg/fg (chrome) plus accent/dim/panel — so
// styles.css can consume the REAL theme roles instead of color-mix guesses, and
// every non-default theme's chat colours track the daemon live. Each var is set
// only when its value is a valid hex, so a partial/legacy push never clobbers a
// role with garbage (the CSS fallback holds).
function applyPaletteVars(palette: PaletteColors) {
  if (typeof document === 'undefined') return
  const root = document.documentElement.style
  const setVar = (name: string, val: string | undefined) => {
    if (val && HEX_RE.test(val)) root.setProperty(name, val)
  }
  setVar('--koma-bg', palette?.bg)
  setVar('--koma-fg', palette?.fg)
  setVar('--koma-accent', palette?.accent)
  setVar('--koma-dim', palette?.dim)
  setVar('--koma-panel', palette?.panel)
}

// Basename of a path — a diff tab's title (TabBar disambiguates colliding
// basenames with a dim parent-dir suffix at render time).
function tabBaseName(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

export const useKoma = create<KomaState>((set, get) => ({
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,
  ui: initialUi,
  config: initialConfig,
  modelList: initialModelList,
  routeList: initialRouteList,
  settingsValues: null,
  usagePreview: null,

  push: (env) => {
    switch (env.k) {
      case 'Snapshot':
        set((s) => {
          // A Snapshot whose session id differs from the current one is a
          // session SWITCH. The in-flight stream/reasoning carried over via
          // `...s.session` belongs to the OLD session — drop it so the old
          // half-rendered reply doesn't bleed into the new view until the
          // next send clears it.
          const switched = env.session !== s.session.id
          return {
            session: {
              ...s.session,
              id: env.session,
              state: env.state,
              messages: env.messages,
              title: env.title,
              subagents: env.subagents,
              bash: env.bash,
              // Defensive fallback: tolerates a host build that hasn't started
              // projecting fileChanges[] on the Snapshot envelope yet.
              fileChanges: env.fileChanges ?? [],
              // Defensive fallback: tolerates a host build that hasn't started
              // projecting planTodos[] on the Snapshot envelope yet, and a host
              // build that projects rows without the newer `locked` flag.
              planTodos: (env.planTodos ?? []).map((t) => ({ ...t, locked: t.locked ?? false })),
              // Defensive fallback: tolerates a host build that hasn't started
              // projecting attachments[] on the Snapshot envelope yet.
              attachments: env.attachments ?? [],
              // Adopt the projected agent mode when present; keep the current
              // one otherwise (host build not projecting it yet).
              mode: env.mode ?? s.session.mode,
              // Adopt the projected pending-steer queue; defensive fallback for a
              // host build that doesn't project it yet (empty queue).
              pendingSteer: env.pendingSteer ?? [],
              // Adopt the projected approval gate; defensive fallbacks for a host
              // build that doesn't project it yet (gate closed, no pending call).
              awaitingApproval: env.awaitingApproval ?? false,
              approvalReason: env.approvalReason ?? null,
              pendingCall: env.pendingCall ?? null,
              ...(switched ? { stream: '', reasoning: '' } : {}),
            },
            palette: env.palette,
            // Any Snapshot is authoritative proof the swap (if one was in
            // flight) has landed — clear the loader. A genuine session SWITCH
            // additionally resets the editor tabs back to just the chat tab: a
            // diff tab is file-change context for the OLD session, so it must
            // not bleed across.
            ui: {
              ...s.ui,
              switchingTo: null,
              ...(switched ? { tabs: [makeChatTab()], activeTabId: 'chat' } : {}),
            },
          }
        })
        applyPaletteVars(env.palette)
        break
      case 'Switching':
        set((s) => {
          // Prefer an optimistic label ResumePalette already raised (the
          // friendly name the user clicked); otherwise resolve the target id
          // against the hub rows; else fall back to a generic label (e.g. a
          // daemon-driven new session with no hub row yet). Never clobber a
          // nicer label with a raw uuid.
          if (s.ui.switchingTo) return s
          const row =
            s.hub.cooking.find((c) => c.id === env.to) ??
            s.hub.history.find((h) => h.id === env.to)
          return { ui: { ...s.ui, switchingTo: row?.name ?? 'session' } }
        })
        break
      case 'StreamMsg':
        set((s) => ({ session: { ...s.session, stream: env.text } }))
        break
      case 'Reasoning':
        set((s) => ({ session: { ...s.session, reasoning: env.text } }))
        break
      case 'Status':
        set((s) => {
          // Only raise a NEW toast when the text actually changed from the one
          // already showing — the host re-pushes the same live toast on every
          // Status tick (it has a host-side TTL), so deduping by text keeps the
          // dismiss timer from being reset on each tick. A cleared toast
          // (env.toast null) never wipes an active card; the auto-dismiss owns
          // that so a working=false status can't cut a toast short.
          const raise = !!env.toast && env.toast !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return {
            session: {
              ...s.session,
              working: env.working,
              // Usage counters + mode ride the Status envelope too (not just
              // Snapshot), so the footer updates live mid-turn. Optional-
              // tolerant: an older host build omits these — keep the current
              // value rather than resetting to 0/'auto' on every tick.
              tokensIn: env.tokensIn ?? s.session.tokensIn,
              tokensCached: env.tokensCached ?? s.session.tokensCached,
              tokensOut: env.tokensOut ?? s.session.tokensOut,
              cost: env.cost ?? s.session.cost,
              mode: env.mode ?? s.session.mode,
            },
            ui: raise
              ? {
                  ...s.ui,
                  toastSeq: seq,
                  toast: { id: seq, text: env.toast as string, kind: env.toastKind === 'error' ? 'error' : 'info' },
                }
              : s.ui,
          }
        })
        break
      case 'Hub':
        set((s) => ({
          hub: { ...s.hub, state: env.state, cooking: env.cooking, history: env.history },
          // Deterministic failure-recovery clear: host_swapper pushes a fresh
          // Hub on EVERY path back to the swapper, including the
          // attach-failure/degrade path (which never emits a Snapshot). A
          // valid in-flight swap can't produce a spurious Hub here either —
          // ResumePalette (the only source of RefreshHub) is unmounted by
          // startSwitching's caller before the request is sent, so its
          // RefreshHub polling interval is already torn down. Net: any Hub
          // that arrives while switchingTo is set means the swap bounced
          // back to the hub, so clear the loader unconditionally.
          ui: { ...s.ui, switchingTo: null },
        }))
        break
      case 'SearchResults':
        set((s) => ({ session: { ...s.session, searchResults: env.items } }))
        break
      case 'Config':
        // Empty/swapper theme: Config is pushed in BOTH the empty/swapper
        // state and the attached state, so it's the reliable carrier for the
        // active palette — adopt it (store + CSS vars) whenever present, same
        // consumption path as the Snapshot palette. No-op when omitted.
        if (env.palette) applyPaletteVars(env.palette)
        set((s) => ({
          config: {
            mcp: env.mcp,
            providers: env.providers,
            models: env.models,
            loaded: true,
            // Preserve the derived-default when the host omits these (keeps the
            // theme picker populated + the gate on the config-inference path).
            firstRun: env.firstRun,
            theme: env.theme ?? s.config.theme,
            themes: env.themes && env.themes.length > 0 ? env.themes : s.config.themes,
            // Adopt the resolved palette catalogue when present; keep the current
            // one otherwise (host build not projecting it yet).
            palettes: env.palettes && env.palettes.length > 0 ? env.palettes : s.config.palettes,
          },
          ...(env.palette ? { palette: env.palette } : {}),
        }))
        break
      case 'ModelList':
        set(() => ({ modelList: env.models }))
        break
      case 'RouteList':
        set(() => ({
          routeList: { provider: env.provider, modelId: env.modelId, routes: env.routes },
        }))
        break
      case 'FileDiff':
        set((s) => {
          const id = `diff:${env.path}`
          // Ignore a reply for a tab closed while the req was in flight.
          if (!s.ui.tabs.some((t) => t.id === id)) return s
          return {
            ui: {
              ...s.ui,
              tabs: s.ui.tabs.map((t) =>
                t.id === id && t.kind === 'diff'
                  ? {
                      ...t,
                      loading: false,
                      diff: {
                        original: env.original,
                        modified: env.modified,
                        error: env.error,
                        binary: env.binary,
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'SettingsValues':
        set(() => ({
          settingsValues: {
            name: env.name,
            workdir: env.workdir,
            shortSend: env.shortSend,
            slidingCache: env.slidingCache,
            bashSaving: env.bashSaving,
            internetMode: env.internetMode,
            palette: env.palette,
          },
        }))
        break
      case 'UsagePreview':
        set((s) => {
          // Drop a reply for a scope the user has since switched away from (a
          // rapid all/session toggle racing an in-flight request) — leave
          // `usagePreview` as-is (likely null, showing the loading row) until
          // the reply matching the CURRENT scope lands.
          if (env.scope !== s.ui.usageScope) return s
          return {
            usagePreview: {
              cost: env.cost,
              tokensIn: env.tokensIn,
              tokensCached: env.tokensCached,
              tokensOut: env.tokensOut,
              calls: env.calls,
              days: env.days,
              topModels: env.topModels,
            },
          }
        })
        break
    }
  },

  req: (g) => {
    try {
      window.ipc?.postMessage(JSON.stringify({ t: 'req', ...g }))
    } catch {
      /* ipc unavailable */
    }
  },

  openOmniSearch: () => set((s) => ({ ui: { ...s.ui, omnisearchOpen: true } })),
  closeOmniSearch: () => set((s) => ({ ui: { ...s.ui, omnisearchOpen: false } })),
  insertToComposer: (path) => set((s) => ({ ui: { ...s.ui, composerInsert: path } })),
  consumeComposerInsert: () => set((s) => ({ ui: { ...s.ui, composerInsert: null } })),
  refillComposer: (text) => set((s) => ({ ui: { ...s.ui, composerRefill: text } })),
  consumeComposerRefill: () => set((s) => ({ ui: { ...s.ui, composerRefill: null } })),
  stageRewind: (index) => set((s) => ({ ui: { ...s.ui, pendingRewindIndex: index } })),
  clearRewind: () => set((s) => ({ ui: { ...s.ui, pendingRewindIndex: null } })),
  requestScrollBottom: () => set((s) => ({ ui: { ...s.ui, scrollTick: s.ui.scrollTick + 1 } })),
  startSwitching: (name) => set((s) => ({ ui: { ...s.ui, switchingTo: name } })),
  cancelSwitching: () => set((s) => ({ ui: { ...s.ui, switchingTo: null } })),
  dismissToast: (id) =>
    set((s) => (s.ui.toast?.id === id ? { ui: { ...s.ui, toast: null } } : s)),
  openSettingsTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'settings')
      const tabs: Tab[] = exists
        ? s.ui.tabs
        : [...s.ui.tabs, { id: 'settings', kind: 'settings' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'settings' } }
    })
    get().req({ r: 'GetSettings' })
  },
  openDiffTab: (path) => {
    const id = `diff:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) =>
            t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t,
          )
        : [...s.ui.tabs, { id, kind: 'diff', path, title: tabBaseName(path), loading: true }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'FileDiff', path })
  },
  closeTab: (id) =>
    set((s) => {
      if (id === 'chat') return s
      const idx = s.ui.tabs.findIndex((t) => t.id === id)
      if (idx < 0) return s
      const tabs = s.ui.tabs.filter((t) => t.id !== id)
      // If the closed tab was active, fall back to the left neighbour. idx-1 is
      // always valid (tabs[0] is the chat tab), so this never underflows.
      const activeTabId =
        s.ui.activeTabId === id ? s.ui.tabs[idx - 1]?.id ?? 'chat' : s.ui.activeTabId
      return { ui: { ...s.ui, tabs, activeTabId } }
    }),
  activateTab: (id) => {
    const tab = get().ui.tabs.find((t) => t.id === id)
    if (!tab) return
    const isDiff = tab != null && tab.kind === 'diff'
    set((s) => ({
      ui: {
        ...s.ui,
        activeTabId: id,
        // Mark a re-focused diff tab loading for the re-request below, but keep
        // its existing `diff` so the editor doesn't flash to a spinner.
        tabs: isDiff
          ? s.ui.tabs.map((t) =>
              t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t,
            )
          : s.ui.tabs,
      },
    }))
    if (isDiff && tab.kind === 'diff') get().req({ r: 'FileDiff', path: tab.path })
    // Re-focusing the Settings tab re-requests its values so they're fresh (the
    // name/workdir may have changed via other paths, e.g. the RenameOverlay).
    if (tab.kind === 'settings') get().req({ r: 'GetSettings' })
  },
  focusPlanSection: () => set((s) => ({ ui: { ...s.ui, focusPlanTick: s.ui.focusPlanTick + 1 } })),
  setUsageScope: (scope) => set((s) => ({ ui: { ...s.ui, usageScope: scope } })),
}))
