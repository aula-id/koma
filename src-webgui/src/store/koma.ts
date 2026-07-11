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
  warn: string
  success: string
  info: string
  error: string
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
  // The foreground session's stored `/effort` value ("" = model default), for
  // the composer EffortPicker's trigger-pill label.
  effort: string
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

// One phase of the TUI-parity startup splash (host `Loading` push envelope) —
// mirrors the TUI's cold-session warm-up phase lines ("indexing workspace" /
// "reading project docs"). 'pending' = not started yet, 'running' = in
// progress, 'done'/'skipped'/'failed' are terminal.
export type LoadPhase = 'pending' | 'running' | 'done' | 'skipped' | 'failed'

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
  source: 'session' | 'global' | 'builtin'
  modelUuid: string | null
  // Host-resolved display name for `modelUuid` (informational only — the
  // Agents panel/tab re-resolve `modelUuid` against `catalogueModels`/
  // `catalogueProviders` themselves for the "name @ provider" label, per the
  // locked design, rather than trusting this field's exact format).
  model: string | null
  tools: string[]
  prompt: string
}

// One file entry in a GitStatus's staged/unstaged list — mirrors the host's
// `GitFileEntry` (git.rs, `rename_all = "camelCase"`). `status` is a single
// git-porcelain status character ("M"/"A"/"D"/"R"/"C"/"U"/"?") the GIT panel
// renders as a badge. `origPath` is non-null only for a rename/copy record
// (shown as `origPath -> path`). A single on-disk path can legitimately
// appear in BOTH `staged` and `unstaged` (e.g. `MM`) — not a bug.
export type GitFileEntry = {
  path: string
  origPath: string | null
  status: string
  staged: boolean
}

// The Source Control "GIT" panel's authoritative status (host `GitStatus`
// push, mirrors `GitStatusResult` verbatim). `error` set means the working
// directory isn't a git repository (or `git status` failed) — every other
// field then sits at its neutral default. Global (not per-session) in the
// store, mirroring how the host resolves it off the foreground session's
// workdir; refreshed via `refreshGitStatus()`.
export type GitStatus = {
  root: string | null
  branch: string | null
  detached: boolean
  ahead: number | null
  behind: number | null
  staged: GitFileEntry[]
  unstaged: GitFileEntry[]
  error: string | null
  // The SSH vault key (by name) currently assigned to this repo for remote ops
  // (wave 4b), or null when none is assigned (remote ops then use the system
  // default ssh-agent/keys). Mirrors the host's `GitStatusResult.key_name`. The
  // GIT panel's key picker's current selection.
  keyName: string | null
  // Which sequencer op (if any) is currently mid-flight (G5c) — one of
  // "merge"/"cherry-pick"/"revert"/"rebase", or null when the repo is clean.
  // Mirrors the host's `GitStatusResult.in_progress`. Drives the ConflictBanner
  // (Abort/Continue).
  inProgress: string | null
  // The porcelain-v2 unmerged/conflict file records (G5c), split out of
  // `staged`/`unstaged` — a conflicted file shouldn't masquerade as an ordinary
  // modification. Empty outside a conflict. Mirrors the host's
  // `GitStatusResult.conflicted`.
  conflicted: GitFileEntry[]
}

// ---- Commit graph (G2) wire types — mirror the host's git_graph.rs DTOs
// (every struct `rename_all = "camelCase"`), matched field-for-field so a wire
// mismatch can't silently read `undefined` at runtime. -----------------------

// One branch entry in a BranchList reply — host `BranchInfo` (git_branch.rs,
// `rename_all = "camelCase"`) — G4. `kind` is "local" (`refs/heads/…`),
// "remote" (`refs/remotes/…`, e.g. `origin/main`), or "tag" (`refs/tags/…` —
// GK4a, listed alongside branches for the React ref-tree); `isCurrent` marks
// the single branch HEAD currently points at (never true for a remote or tag
// entry).
export type BranchInfo = {
  name: string
  kind: 'local' | 'remote' | 'tag'
  isCurrent: boolean
}

// One repo entry in a RepoList reply — host `RepoEntry` (multi-repo support,
// `rename_all = "camelCase"`). `root` is the repo's absolute workdir root
// (also the `SetActiveRepo` request's `root` value); `name` is its display
// label (basename) for the repo picker.
export type RepoEntry = { root: string; name: string }

// One stash entry in a StashList reply — host `StashEntry` (git_stash.rs,
// `rename_all = "camelCase"`) — GK4c. `index` is the `stash@{N}` slot number
// (0 is the most-recently-pushed stash); `message` is everything after the
// `stash@{N}: ` marker verbatim (covers both git's default "WIP on <branch>:
// …" message and a custom `git stash push -m <msg>`).
export type StashEntry = {
  index: number
  message: string
}

// One ref (branch/tag/HEAD pointer) decorating a commit — host `GitRef`. `kind`
// classifies it off the FULL ref path host-side (a distinct chip colour per
// kind); `isHead` marks the single `HEAD -> …` current-branch pointer.
export type GitRef = {
  name: string
  kind: 'head' | 'local' | 'remote' | 'tag'
  isHead: boolean
}

// One commit row in a GitGraph reply — host `GitCommitNode`. The list is
// newest-first, already `--date-order --parents`; `parents` is empty for a root
// commit; `refs` is empty for the common case of a commit nothing points at.
export type GitCommitNode = {
  sha: string
  parents: string[]
  refs: GitRef[]
  author: string
  email: string
  date: string
  subject: string
}

// One commit row in an Activity reply (GK5b) — host `ActivityCommit`. `date`
// is the author date as an ISO-8601 string (parsed client-side, never
// host-side); `added`/`deleted` are the commit's SUMMED line counts across
// every changed file (binary files contribute 0 to both).
export type ActivityCommit = {
  sha: string
  author: string
  email: string
  date: string
  added: number
  deleted: number
}

// One changed-file entry in a CommitDetail — host `CommitFile`. `status` is
// git's own token ("M"/"A"/"D"/"R100"/…); `origPath` is non-null only on a
// rename/copy record (then `path` is the NEW path, `origPath` the OLD one).
export type CommitFile = {
  status: string
  path: string
  origPath: string | null
}

// A single commit's full metadata (incl. body) + first-parent changed-file list
// — host `CommitDetailResult`. `error` non-null means the sha failed validation
// or the workdir isn't a git repo (every other field then a neutral default).
// Stored into the graph slice's `detail` by the CommitDetail push.
export type CommitDetail = {
  sha: string
  author: string
  email: string
  date: string
  subject: string
  body: string
  parents: string[]
  files: CommitFile[]
  error: string | null
}

// One keypair entry in the Settings "SSH Keys" section's vault list — mirrors
// the host's `KeyInfo` (keys.rs, `rename_all = "camelCase"`). This is a
// GUI-only, manual, user-owned key vault (`<~/.koma>/keys/`), completely
// separate from the model's own git credential machinery.
export type KeyInfo = {
  name: string
  fingerprint: string
  comment: string
  keyType: string
}

// A one-shot reveal of a keypair's contents (KeyReveal push) — the SSH Keys
// section's transient "Copy public key" / "Reveal private key" result, kept
// separate from the authoritative `keys` list. `private` echoes which half
// was read; `error` set means `content` is empty.
export type KeyReveal = {
  name: string
  private: boolean
  content: string
  error: string | null
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

// The OAuth login screen's state machine phase (host `OAuthState.phase`).
// 'idle' = list/picker view; 'starting' = flow spawning (spinner); 'waiting_url'
// = codex-style PKCE (browser already opened daemon-side, `url` is the
// fallback/status link); 'waiting_code' = kilocode-style device flow
// (`userCode`/`verificationUrl`); 'paste' = manual access-token entry;
// 'success'/'failed' are terminal (conns updated / `error` set respectively).
export type OAuthPhase = 'idle' | 'starting' | 'waiting_url' | 'waiting_code' | 'paste' | 'success' | 'failed'

// One persisted OAuth connection (host `OAuthConnWire` — a deliberately
// TOKENLESS projection; no access/refresh/id token ever crosses the bridge).
// NOTE snake_case on the wire (`account_id`), like AgentEntry's nested
// structs — the push case normalizes it to `accountId` below.
export type OAuthConn = {
  uuid: string
  name: string
  // Wire provider token, e.g. "codex" | "kilocode" — matches an
  // OAuthProviderEntry.id when that provider is still available.
  provider: string
  email: string
  plan: string
  accountId: string
}

// One available OAuth login provider (host `OAuthProviderWire`) — DATA-DRIVEN,
// never hardcode this list client-side (it's designed to grow). `kind`
// distinguishes the flow shape: 'pkce' (browser redirect), 'device' (user
// code), 'paste' (manual token) — typed as a bare `string` (not a closed
// union) so an unforeseen future kind degrades to a generic render instead of
// a type error.
export type OAuthProviderEntry = { id: string; label: string; kind: string }

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
  // The singleton Help page — a static, wire-free reference for the GUI's own
  // features (composer/sessions/tabs/keyboard). Opened from the ActivityBar's
  // (?) button, directly above Settings. Deduped by the fixed id 'help';
  // closeable like a diff tab. Mirrors the Settings tab's plumbing exactly.
  | { id: 'help'; kind: 'help' }
  | {
      // Stable id — `diff:${path}` for a File-changed diff (find-by-path is
      // trivial), or `gitdiff:${staged ? 'staged' : 'unstaged'}:${path}` for a
      // GIT-panel diff (a git diff needs BOTH a staged and unstaged tab for
      // the SAME path open side by side, so `diff:${path}` alone would
      // collide — the `staged`/`unstaged` segment disambiguates).
      id: string
      kind: 'diff'
      // The path exactly as the fileChanges record (or GitFileEntry) carries
      // it — the key for the FileDiff/GitDiff req + reply.
      path: string
      // Basename of `path`. TabBar adds a dim parent-dir suffix at render time
      // when two open tabs share a basename (collision depends on the live tab
      // set, so it's resolved there, not baked into the stored title).
      title: string
      // Filled by the FileDiff/GitDiff reply; undefined until the first reply
      // lands.
      diff?: DiffPayload
      // True while a FileDiff/GitDiff req is in flight (initial open OR a
      // re-request on re-activate). A stale `diff` keeps rendering while
      // loading so re-focus never flashes to a spinner.
      loading: boolean
      // Present ONLY on a GIT-panel diff tab (opened via `openGitDiffTab`):
      // `true` = staged (index vs HEAD), `false` = unstaged (worktree vs
      // index). Undefined on a plain File-changed diff tab — this is what
      // `activateTab`'s re-request routes on (GitDiff vs FileDiff).
      staged?: boolean
      // Present ONLY on a commit-graph diff tab (opened via `openCommitDiffTab`):
      // the commit sha whose first-parent diff this tab shows. Distinct tab-id
      // scheme (`commitdiff:${sha}:${path}`) from the File-changed (`diff:`) and
      // GIT-panel (`gitdiff:`) schemes, so a commit-history diff never collides
      // with either. `activateTab`'s re-request checks this FIRST (GitCommitDiff)
      // before the `staged` (GitDiff) / plain (FileDiff) branches — a commit-diff
      // tab has no `staged`, so it would otherwise wrongly re-fire FileDiff.
      commitSha?: string
    }
  // A read-only STREAM tab live-streaming ONE sub-agent's transcript. Stable id
  // `sa:${agentId}` so open/dedupe is trivial. Content is NOT stored on the tab — the
  // StreamTab reads the live entry from `session.subagents` by `agentId` (so it updates
  // as the host pushes fresh transcript). `title` is the agent name at open time.
  | { id: string; kind: 'subagent'; agentId: number; title: string }
  // A read-only STREAM tab live-streaming ONE bash job's output. Stable id
  // `bash:${jobId}`; content read live from `session.bash` by `jobId`. `title` is the
  // (truncated) command.
  | { id: string; kind: 'bash'; jobId: number; title: string }
  // Per-agent editor tab (Agents sidebar panel). `agentId` is the agent's
  // NAME for an edit, `null` for a create — NOT the settings/help singleton
  // pattern: open-or-focus is keyed PER agentId (two different agents' editors
  // can be open side-by-side; re-clicking the same agent's row just focuses
  // its existing tab), matched by `agentId`, NOT by `id`. `id` is a client-
  // minted, STABLE identifier independent of `agentId` — deliberately, so a
  // successful create/rename (which mutates `agentId` via `renameAgentTab`)
  // never changes this tab's React `key` (`key={t.id}` in TabbedMain) and
  // never forces a remount that would wipe in-progress edits held in the
  // component's local state right as Save fires. Closeable like a diff tab;
  // closing an unsaved tab discards silently (no local draft is ever
  // persisted to the store).
  | { id: string; kind: 'agent'; agentId: string | null }
  // The singleton GitKraken-style commit-graph tab (id 'graph'), opened from the
  // Source Control panel header. Deduped by the fixed id; closeable like a diff
  // tab. Content (commits/selection/detail) lives in the `graph` store slice, not
  // on the tab — the GraphTab reads it live and fires refreshGraph on mount.
  | { id: 'graph'; kind: 'graph' }

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
      origin?: 'git' | 'baseline'
    }
  // Reply to GuiReq UsagePreview — a LAST-7-DAYS usage preview computed straight
  // off the global usage ledger (host-only, never touches the daemon). ALWAYS a
  // reply so the Usage panel's loading state can never hang. `scope` echoes the
  // request's "all"/"session" token, and `sessionId` echoes the session uuid
  // ACTUALLY queried (null for an "all" scope) — together they let the reducer drop
  // a reply that no longer matches what's currently selected/attached: a rapid
  // all/session toggle racing an in-flight request (scope mismatch), OR the
  // foreground session switching mid-flight while "session" scope stayed selected
  // (session id mismatch — otherwise session A's numbers would render under B's
  // attach).
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
      sessionId: string | null
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
      effort: string
    }
  // Reply to GuiReq GetEffortOptions — the composer EffortPicker's derived
  // `/effort` menu for the foreground session's current model. ALWAYS a reply
  // (loading/unsupported/ready) so the picker never hangs.
  | { k: 'EffortOptions'; options: string[]; selected: number; note: string; state: 'loading' | 'unsupported' | 'ready' }
  // TUI-parity startup splash (cold-session warm-up): the host's two
  // background warm-up phases — indexing the workspace and reading project
  // docs (awareness). `active` false means no warm-up in flight (or it just
  // finished) — the reducer clears `ui.loading` to null in that case rather
  // than storing a "false" splash. Pushed independently of Snapshot/Switching
  // so the splash can keep showing (and finish its phase lines) even after
  // the attach itself has landed and `ui.switchingTo` has already cleared.
  | { k: 'Loading'; active: boolean; workspace: LoadPhase; awareness: LoadPhase }
  // Reply to GuiReq GetAgents (and the re-push after every SetAgent/
  // DeleteAgent) — the Agents dashboard's full agent list + model/provider
  // catalogues. ALWAYS a reply, even un-attached (host answers from built-in +
  // global config only). MIXED CASING, verified against the Rust wire: the
  // envelope's OWN fields are camelCase (`agents`/`catalogueModels`/
  // `catalogueProviders`/`availableTools`), but each nested entry struct has
  // NO rename_all of its own and serializes plain snake_case (`model_uuid`,
  // `model_id`, `provider_uuid`) — NOT a typo, the push case normalizes these
  // into the camelCase `AgentEntry`/`CatalogueModelEntry`/
  // `CatalogueProviderEntry` shapes above. `availableTools` is optional-
  // tolerant (defaults to [] in the reducer) for an older host build that
  // doesn't project it yet — the AgentTab tools chip grid then just has
  // nothing to offer beyond whatever an existing agent already carries.
  | {
      k: 'AgentsValues'
      reqSeq: number // 0 = no correlation (read-only fetch / host-built fallback)
      agents: {
        name: string
        description: string
        conditions: string
        source: string
        model_uuid: string | null
        model: string | null
        tools: string[]
        prompt: string
      }[]
      catalogueModels: { uuid: string; name: string; model_id: string; provider_uuid: string }[]
      catalogueProviders: { uuid: string; name: string; endpoint: string }[]
      availableTools?: string[]
    }
  // Reply to GuiReq GetOAuthState (and the re-push after every StartOAuth
  // progress tick / SubmitOAuthPaste / CancelOAuth / DeleteOAuthConn) — the
  // OAuth login screen's full state: which phase the flow is in + its
  // phase-specific fields + the connections/providers lists. ALWAYS a reply to
  // GetOAuthState, even un-attached (host answers from disk + the provider
  // registry). Same MIXED CASING as AgentsValues: the envelope's OWN fields
  // are camelCase (`userCode`/`verificationUrl`), but the nested `conns`/
  // `providers` entry structs have no rename_all of their own and serialize
  // plain snake_case (`account_id`) — normalized to camelCase `OAuthConn` in
  // the push case.
  | {
      k: 'OAuthState'
      phase: string
      url: string | null
      userCode: string | null
      verificationUrl: string | null
      error: string | null
      conns: { uuid: string; name: string; provider: string; email: string; plan: string; account_id: string }[]
      providers: { id: string; label: string; kind: string }[]
    }
  // Reply to GuiReq GitStatus — host-computed branch/ahead/behind + staged/
  // unstaged file lists for the Source Control "GIT" panel. Carries the
  // Rust `GitStatusResult` verbatim (already camelCase) flattened onto the
  // envelope (a `#[serde(tag = "k")]` newtype variant). ALWAYS a reply so the
  // panel never hangs loading — `error` set means not a git repository.
  | {
      k: 'GitStatus'
      root: string | null
      branch: string | null
      detached: boolean
      ahead: number | null
      behind: number | null
      staged: GitFileEntry[]
      unstaged: GitFileEntry[]
      error: string | null
      keyName: string | null
      // G5c additions — see GitStatus type comments.
      inProgress: string | null
      conflicted: GitFileEntry[]
    }
  // Reply to GuiReq GitDiff — a host-computed git diff for one GIT-panel file
  // row, for a Monaco diff tab. `staged` echoes the request (index-vs-HEAD vs
  // worktree-vs-index) so the reducer applies it to the matching
  // `gitdiff:${staged}:${path}` tab, never the wrong one.
  | {
      k: 'GitDiff'
      path: string
      staged: boolean
      original: string
      modified: string
      error: string | null
      binary: boolean
    }
  // Reply to a GitStage/GitUnstage/GitDiscard/GitCommit/GitFetch/GitPull/GitPush
  // mutation. `op` is "stage"/"unstage"/"discard"/"commit"/"fetch"/"pull"/"push";
  // `error` (only when `ok` is false) is git's own failure message. `message`
  // (wave 4b remote ops only — a short human-readable SUCCESS summary, e.g. a
  // fetch/pull/push's own stdout/stderr) is present only when the host had
  // something worth surfacing; absent (undefined) for every local mutation and
  // for a remote op with nothing to say. Carries no list data — ALWAYS
  // immediately followed by a fresh GitStatus push, which is what actually
  // refreshes the panel.
  | {
      k: 'GitOp'
      ok: boolean
      op: string
      error: string | null
      message?: string
    }
  // Reply to GuiReq GitGraph — a host-computed paginated commit graph across
  // every ref (GitKraken-style tab). Carries `GitGraphResult` verbatim (already
  // camelCase) flattened onto the envelope (a `#[serde(tag = "k")]` newtype
  // variant). `head` is the current HEAD sha (null when unresolved); `hasMore`
  // hints more history exists past this page (scroll-load-more). `error` set
  // means not a git repository (`commits` then empty). ALWAYS a reply.
  | {
      k: 'GitGraph'
      commits: GitCommitNode[]
      head: string | null
      hasMore: boolean
      error: string | null
    }
  // Reply to GuiReq GitCommitDetail — one commit's full metadata (incl. body) +
  // first-parent changed-file list, for the graph's detail pane. Carries
  // `CommitDetailResult` verbatim (already camelCase) flattened onto the
  // envelope. `sha` echoes the request so the reducer can drop a stale reply for
  // a since-changed selection.
  | {
      k: 'CommitDetail'
      sha: string
      author: string
      email: string
      date: string
      subject: string
      body: string
      parents: string[]
      files: CommitFile[]
      error: string | null
    }
  // Reply to GuiReq GitCommitDiff — one file's diff at `sha` vs its first parent,
  // for a Monaco diff tab. SEPARATE envelope + tab-id scheme
  // (`commitdiff:${sha}:${path}`) from GitDiff (working-tree/index) so a
  // commit-history diff never collides with a Source-Control one. `sha`/`path`
  // echo the request so the reducer applies it to the matching tab.
  | {
      k: 'CommitDiff'
      sha: string
      path: string
      original: string
      modified: string
      error: string | null
      binary: boolean
    }
  // Reply to GuiReq KeyList — the Settings "SSH Keys" section's authoritative
  // vault list. ALWAYS a reply so the section never hangs loading (an empty
  // vault is itself a valid "no keys yet" state). Also arrives as the
  // follow-up refresh after any KeyGenerate/KeyImport/KeyDelete mutation.
  | {
      k: 'KeyList'
      keys: KeyInfo[]
    }
  // Reply to GuiReq KeyReveal — a host-computed keypair reveal for the "Copy
  // public key" / "Reveal private key" actions. `private` echoes the request
  // so the reducer never mismatches a public reveal with a private one.
  | {
      k: 'KeyReveal'
      name: string
      private: boolean
      content: string
      error: string | null
    }
  // Reply to a KeyGenerate/KeyImport/KeyDelete mutation. `op` is
  // "generate"/"import"/"delete"; `error` (only when `ok` is false) is the
  // host's own failure message. Carries no list data — ALWAYS immediately
  // followed by a fresh KeyList push, which is what actually refreshes the
  // section's list.
  | {
      k: 'KeyOp'
      ok: boolean
      op: string
      error: string | null
    }
  // Reply to GuiReq GitBranchList (G4) — every local + remote-tracking branch
  // for the branch-switcher popover / graph context menu. Carries
  // `BranchListResult` verbatim (already camelCase) flattened onto the
  // envelope. ALWAYS a reply so the picker never hangs loading.
  | {
      k: 'BranchList'
      branches: BranchInfo[]
      error: string | null
    }
  // Reply to GuiReq GitRepos (multi-repo support) — every detected repository
  // root in the workspace + which one is currently active. Carries
  // `RepoEntry[]` verbatim (already camelCase) flattened onto the envelope.
  // ALWAYS a reply so the picker never hangs loading.
  | { k: 'RepoList'; repos: RepoEntry[]; active: string | null }
  // Reply to GuiReq GitStashList (GK4c) — every `git stash list` entry for the
  // toolbar's Stash/Pop buttons. Carries `StashListResult` verbatim (already
  // camelCase) flattened onto the envelope. ALWAYS a reply so a non-repo
  // workdir just shows an empty (Pop-disabled) list rather than hanging.
  | {
      k: 'StashList'
      entries: StashEntry[]
      error: string | null
    }
  // Reply to GuiReq GitActivity (GK5b) — per-commit author/date/lines-changed
  // rows for the bubble/activity chart. Carries `ActivityResult` verbatim
  // (already camelCase) flattened onto the envelope. `error` set means the
  // workdir isn't a git repository (`commits` then empty). `path` echoes the
  // request's pathspec (`null` for the whole-branch case) so the reducer can
  // drop a stale reply for a since-changed path filter. ALWAYS a reply.
  | {
      k: 'Activity'
      commits: ActivityCommit[]
      path: string | null
      error: string | null
    }
  // Result of a daemon-side SetAgent/DeleteAgent operation (attached path).
  // `ok: false` + `error` surfaces the failure as a toast and clears the
  // AgentTab's saving state. On success the authoritative reply is always a
  // fresh `AgentsValues` push, so this envelope only carries failures — the
  // AgentsValues push handler below is what the AgentTab watches for success.
  | { k: 'AgentOp'; ok: boolean; error: string | null; reqSeq: number }

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

// The OAuth login screen's full state (host `OAuthState` push) — global, not
// per-session (mirrors ConfigSlice). REPLACED wholesale on each push, never
// accumulated. `url`/`userCode`/`verificationUrl`/`error` are only meaningful
// for the phase that produces them (see `OAuthPhase`); the others sit at
// `null` outside their owning phase.
type OAuthSlice = {
  phase: OAuthPhase
  url: string | null
  userCode: string | null
  verificationUrl: string | null
  error: string | null
  conns: OAuthConn[]
  providers: OAuthProviderEntry[]
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
  // TUI-parity startup splash state (host `Loading` push envelope) — drives
  // SwitchingOverlay's cold-session warm-up splash (centered "koma" wordmark +
  // the two phase rows). `null` = no warm-up in flight. Set by the `Loading`
  // envelope case; defensively cleared on a genuine session switch (Snapshot's
  // `switched` branch) and in `detachSession()` so a stale splash from the OLD
  // session/warm-up can never leak into a new attach.
  loading: { active: boolean; workspace: LoadPhase; awareness: LoadPhase } | null
  // Skip-latch: set true when the user dismisses the cold-start splash via
  // the Skip button. Suppresses splash VISIBILITY only — `loading` itself
  // keeps updating with fresh host `Loading` pushes underneath. Reset to
  // false on a genuine session switch/detach so a NEW cold-start splash can
  // still show later.
  loadingDismissed: boolean
}

// The commit-graph tab's slice (G2) — the loaded commit page(s) + selection +
// fetched detail. `loadMode` records how the LAST GitGraph req was issued so the
// reducer knows whether to concat (append) or replace the incoming page. Global
// (mirrors the `git` slice; the host resolves the graph off the foreground
// session's repo). Reset naturally: a session switch closes the tab (tabs reset
// to chat), and reopening remounts GraphTab → a fresh refreshGraph.
type GraphSlice = {
  commits: GitCommitNode[]
  head: string | null
  hasMore: boolean
  loading: boolean
  loadMode: 'replace' | 'append'
  // A refreshGraph() call that landed while a GitGraph request was already in
  // flight (append or replace) — client-side serialization keeps at most one
  // GitGraph request in flight at a time so `loadMode` is never ambiguous
  // between a racing append and replace reply. Set true instead of firing;
  // replayed by the 'GitGraph' reducer once the in-flight reply lands and
  // `loading` clears.
  pendingRefresh: boolean
  selectedSha: string | null
  detail: CommitDetail | null
  // Rail-line (default, the existing virtualized commit list) vs Bubble
  // (GK5's activity view — a placeholder here) mode switch (GK2). Local UI
  // preference, never persisted, never touches the wire.
  graphMode: 'rail' | 'bubble'
}

// The bubble/activity chart's slice (GK5b) — the loaded per-commit activity
// series for whatever path is currently narrowed to (`null` = whole active
// branch). Global (mirrors `graph`; the host resolves it off the foreground
// session's repo). `error` set means the workdir isn't a git repo — `commits`
// is then empty rather than the chart guessing at a stale page.
type ActivitySlice = {
  commits: ActivityCommit[]
  loading: boolean
  error: string | null
  path: string | null
}

type KomaState = {
  session: SessionSlice
  hub: HubSlice
  palette: PaletteColors
  ui: UiSlice
  config: ConfigSlice
  oauth: OAuthSlice
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
  // The composer EffortPicker's latest GetEffortOptions reply. `null` until the
  // first reply lands (the picker shows a loading row); REPLACED wholesale on
  // each reply — the picker clears this to `null` itself right before firing a
  // fresh GetEffortOptions (the open-time re-request), so a stale menu never
  // lingers under a different state.
  effortOptions: EffortOptions | null
  // The activity-bar Usage panel's latest LAST-7-DAYS preview. `null` until the
  // first reply lands (the panel shows a loading row); REPLACED wholesale on
  // each reply. The panel re-requests it every time it's shown.
  usagePreview: UsagePreview | null
  // Agent-tab saving lifecycle tracker — set right before a SetAgent request,
  // cleared on the confirmatory push. `seq` prevents stale-reply races.
  // `null` when no save is in flight.
  agentSaving: {
    tabId: string
    seq: number
    originalName: string | null
    newName: string
  } | null
  // Session ids with a KillSession/DeleteSession req in flight (ResumePalette
  // / StartScreen row kill/delete confirm) — renders that row non-interactive
  // + spinning instead of its trailing action. Kind-scoped (see `DyingMark`)
  // so a kill mark migrating cooking->history on the next Hub push can't
  // leak onto the row it migrated into. Pruned automatically the moment a
  // fresh Hub push confirms the kill/delete landed, so no explicit "done"
  // signal is needed.
  dyingSessions: DyingMark[]
  // The Agents dashboard's full agent list (built-in + global + session,
  // merged daemon-side) from the latest AgentsValues push. REPLACED wholesale
  // on each push — empty until the first GetAgents reply lands.
  agents: AgentEntry[]
  // The Agents dashboard's model/provider catalogues, from the same push —
  // feeds the AgentTab model picker and the panel row's resolved model label.
  catalogueModels: CatalogueModelEntry[]
  catalogueProviders: CatalogueProviderEntry[]
  // The full set of tool names the daemon knows about — feeds the AgentTab
  // tools field's toggle-chip grid (one chip per available tool). REPLACED
  // wholesale on each AgentsValues push; empty until the first reply lands.
  availableTools: string[]
  // The Source Control "GIT" panel's authoritative status (latest GitStatus
  // push) — global, not per-session (mirrors ConfigSlice; the host resolves
  // it off the foreground session's workdir). REPLACED wholesale on each
  // push; starts at the neutral "no repo" default until the first reply.
  git: GitStatus
  // Which remote sync op (Fetch/Pull/Push) is currently in flight, or null when
  // none is — a transient (not host-authoritative) flag so the GIT panel's sync
  // toolbar can disable its buttons + spinner the active one. Set by
  // `gitFetch`/`gitPull`/`gitPush` right before firing the req; cleared by the
  // matching `GitOp` push reply (success OR failure — either way the op is no
  // longer in flight).
  remoteBusy: string | null
  // Controlled draft text for the GIT panel's commit box. Store-level (not
  // component state) so navigating away from Source Control and back doesn't
  // lose an in-progress message; cleared automatically on a successful commit
  // (see the push reducer's 'GitOp' case).
  commitDraft: string
  // The Settings "SSH Keys" section's authoritative vault list (latest KeyList
  // push) — a GUI-only, manual, user-owned key vault, entirely separate from
  // the model's own git credential machinery. REPLACED wholesale on each push;
  // empty until the first reply lands. Global (not per-session), mirroring `git`.
  keys: KeyInfo[]
  // The SSH Keys section's transient "Copy public key" / "Reveal private key"
  // result (latest KeyReveal push), or `null` when nothing has been revealed
  // yet / the reveal box was dismissed. Kept separate from `keys` (the list
  // itself never carries key material). Named distinctly from the `keyReveal`
  // ACTION below (same-name field+method would collide in this one object type).
  keyRevealResult: KeyReveal | null
  // The branch-switcher popover (footer/GitPanel) + graph context menu's
  // authoritative branch list (latest BranchList push) — every local +
  // remote-tracking branch, current one flagged. REPLACED wholesale on each
  // push; empty until the first reply lands. Global (not per-session),
  // mirroring `git`/`keys` (G4).
  branches: BranchInfo[]
  // Transient (not host-authoritative) picker-loading flag: `true` from
  // `refreshBranches()` until the matching `BranchList` reply lands, so the
  // popover can show a spinner instead of a stale/empty list.
  branchesLoading: boolean
  // Every detected repository root in the workspace (multi-repo support) —
  // latest RepoList push. REPLACED wholesale on each push; empty until the
  // first reply lands. Global (not per-session), mirroring `branches`.
  repos: RepoEntry[]
  // The repo picker's currently-active root (latest RepoList push's `active`
  // field), or `null` when no repo has been detected/selected yet. Drives
  // which repo `git`/`graph`/`activity` describe.
  activeRepoRoot: string | null
  // The toolbar's authoritative stash list (latest StashList push, GK4c) —
  // every `git stash list` entry, newest (index 0) first. REPLACED wholesale
  // on each push; empty until the first reply lands. Global (not
  // per-session), mirroring `branches`. Drives the Pop button's
  // enabled-state + count badge.
  stashes: StashEntry[]
  // Rust -> JS: apply an authoritative push envelope. Always REPLACES the
  // relevant slice fields — never accumulates/appends.
  // The GitKraken-style commit-graph tab's slice (G2). See GraphSlice.
  graph: GraphSlice
  // The bubble/activity chart's slice (GK5b). See ActivitySlice.
  activity: ActivitySlice
  // Open (or focus) the singleton commit-graph tab (id 'graph'). The GraphTab
  // itself fires refreshGraph on mount, so opening is enough. Mirrors
  // openSettingsTab's dedupe + activate shape.
  openGraphTab: () => void
  // (Re)load the FIRST page of the commit graph (replace mode): mark loading +
  // GitGraph{ limit:200, skip:0 }. Fired on GraphTab mount + its refresh button.
  refreshGraph: () => void
  // Append the NEXT page (append mode, skip = current commit count) when scrolled
  // near the bottom and `hasMore`. Guarded against a duplicate in-flight load via
  // the `loading` flag, and a no-op past the last page.
  loadMoreGraph: () => void
  // Select a commit (graph row / a parent-chip click): set selectedSha + fetch
  // its GitCommitDetail. Clears stale `detail` when the sha actually changes so
  // the detail pane shows a loading state, not the previous commit's detail.
  selectCommit: (sha: string) => void
  // Clear the current commit selection (closes the right detail pane): resets
  // `selectedSha`/`detail` back to null without touching the wire.
  clearSelection: () => void
  // Rail-line/Bubble mode switch (GK2) — local UI toggle, no wire request.
  setGraphMode: (mode: 'rail' | 'bubble') => void
  // (Re)load the bubble/activity chart's commit series (GK5b): mark loading +
  // GitActivity{ path, limit:800 }. `path` narrows to one pathspec; omitted or
  // `null` means the whole active branch. Fired on GraphBubble mount + its
  // path-filter submit.
  refreshActivity: (path?: string | null) => void
  // Open (or focus) a Monaco diff tab for `path` at commit `sha` vs its first
  // parent — distinct `commitdiff:${sha}:${path}` id from openDiffTab/
  // openGitDiffTab (never collides). Marks loading + fires the GitCommitDiff req.
  openCommitDiffTab: (sha: string, path: string) => void
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
  dismissLoading: () => void
  // Dismiss the active toast (auto-dismiss timer, or a manual close). No-op if
  // the id no longer matches the current toast (a newer toast already replaced
  // it — its own timer owns the dismissal).
  dismissToast: (id: number) => void
  // Open (or focus) the singleton Settings tab (id 'settings'): find-or-create,
  // activate it, and fire GetSettings so its values refresh. Mirrors openDiffTab's
  // dedupe + activate shape.
  openSettingsTab: () => void
  // Open (or focus) the singleton Help tab (id 'help'): find-or-create, activate
  // it. No wire request — the Help tab is static content, unlike Settings.
  openHelpTab: () => void
  // Open (or focus) a per-agent editor tab: find-or-create keyed by agentId
  // (the agent's name, or `null` for a create — see the Tab union's 'agent'
  // member), activate it. Unlike Settings/Help this is NOT a singleton — a
  // different agentId opens a DIFFERENT tab (diff-tab-style dedupe).
  openAgentTab: (agentId: string | null) => void
  // Rebind an already-open agent tab's identity after a successful
  // create/rename (fired optimistically right after the SetAgent req, since
  // the wire gives no dedicated ack — just a fresh AgentsValues push). Updates
  // both the tab's `id` (so a later click on that agent's now-renamed row in
  // AgentsPanel still finds THIS tab instead of opening a duplicate) and its
  // `agentId`. No-op if `oldAgentId === newAgentId` (a save with no rename).
  renameAgentTab: (oldAgentId: string | null, newAgentId: string) => void
  // Open (or focus) a Monaco diff tab for a File-changed `path`: find-by-path or
  // create, mark it loading, fire the FileDiff req, and activate it. Re-opening
  // an already-open file refreshes it (same loading + re-request path).
  openDiffTab: (path: string) => void
  // Re-fetch the Source Control "GIT" panel's status (branch/ahead-behind +
  // staged/unstaged lists). Fired on GitPanel mount/(re)activation, and once
  // at boot (routes/index.tsx) so the footer's branch indicator populates
  // without ever opening the panel.
  refreshGitStatus: () => void
  // Open (or focus) a Monaco diff tab for a GIT-panel file row: `staged`
  // picks index-vs-HEAD (true) or worktree-vs-index (false) — distinct tab id
  // scheme (`gitdiff:${staged}:${path}`) from `openDiffTab`'s `diff:${path}`,
  // since a git diff needs BOTH staged and unstaged tabs open for the SAME
  // path without colliding. Marks loading + fires the GitDiff req.
  openGitDiffTab: (path: string, staged: boolean) => void
  // Update the GIT panel's commit-box draft text (controlled textarea).
  setCommitDraft: (text: string) => void
  // GIT panel mutations — stage/unstage/discard a batch of repo-root-relative
  // paths (a single path for a row action, every staged/unstaged path for a
  // "Stage All"/"Unstage All"/"Discard All Changes" header action), or commit
  // whatever is currently staged. Each fires the matching req; the reply lands
  // as a one-shot GitOp push (surfaced as an error toast on failure) followed
  // by a fresh GitStatus push that refreshes the panel's lists — these
  // actions never touch `git`/`commitDraft` optimistically themselves.
  gitStage: (paths: string[]) => void
  gitUnstage: (paths: string[]) => void
  gitDiscard: (paths: string[]) => void
  gitCommit: (message: string) => void
  // GIT panel key-picker: assign the repo to vault key `name`, or clear the
  // assignment (`null` — "Default (system ssh)"). No dedicated reply; a fresh
  // GitStatus push (host-side, always follows) reflects the new `keyName`.
  setGitKey: (name: string | null) => void
  // GIT panel sync toolbar: fetch/pull/push the repo's configured remote, using
  // its assigned key's SSH override if one is set. Each sets `remoteBusy` to its
  // op name BEFORE firing the req (disabling the toolbar + showing a spinner on
  // the active button); the matching GitOp reply clears it and toasts the
  // outcome (an error, or a short success confirmation using `message` if
  // present), followed by a fresh GitStatus push that refreshes ahead/behind.
  gitFetch: () => void
  gitPull: () => void
  gitPush: () => void
  // Branch-switcher popover / graph context menu (G4): re-fetch every local +
  // remote-tracking branch. Sets `branchesLoading` before firing the req;
  // cleared by the matching `BranchList` reply.
  refreshBranches: () => void
  // Repo picker (multi-repo support): re-fetch every detected repository root
  // + which one is active.
  refreshRepos: () => void
  // Repo picker pick: switch the active repo to `root`. Optimistically
  // updates `activeRepoRoot` + clears the stale graph/activity slices
  // (preserving the graph view mode) before telling the host — each panel's
  // `activeRepoRoot`-keyed effect then refetches for the newly-active repo.
  setActiveRepo: (root: string) => void
  // Toolbar "Stash" button (GK4c): `git stash push`. Reply lands as a
  // one-shot GitOp push (toasted either way); the GitOp reducer follows up
  // with `refreshStashes()` (the working-tree change itself is already
  // covered by the host's own follow-up GitStatus push, so no explicit
  // refreshGitStatus here). This op never moves HEAD, so no graph refresh.
  gitStash: () => void
  // Toolbar "Pop" button (GK4c): `git stash pop`. May conflict — the
  // existing G5 conflict banner surfaces it via the host's follow-up
  // GitStatus push, same as `gitCherryPick`. Same reply/refresh pattern as
  // `gitStash`.
  gitStashPop: () => void
  // Toolbar mount / stash-op follow-up (GK4c): re-fetch every stash list
  // entry so the Stash/Pop buttons' counts stay correct.
  refreshStashes: () => void
  // Switch (or detach onto) `ref` — a branch name or a sha. SAFE only (never
  // `--force`); the reply lands as a one-shot GitOp push (toasted either way)
  // followed by a fresh GitStatus AND a graph refresh (HEAD moved).
  gitCheckout: (ref: string) => void
  // Create branch `name` from `start` (`null` = current HEAD), optionally
  // switching to it immediately (`checkout`). Same reply pattern as
  // `gitCheckout`.
  gitCreateBranch: (name: string, start: string | null, checkout: boolean) => void
  // Commit-graph row context menu "Cherry-pick commit" (G5c) — may conflict;
  // the follow-up GitStatus push's `inProgress`/`conflicted` carry that state.
  // Reply lands as a one-shot GitOp push (toasted either way), followed by a
  // fresh GitStatus AND graph refresh (see the `GitOp` reducer case).
  gitCherryPick: (sha: string) => void
  // Commit-graph row context menu "Revert commit" (G5c). Same reply pattern
  // as `gitCherryPick`.
  gitRevert: (sha: string) => void
  // Commit-graph row context menu "Reset <branch> to here" (G5c). `mode` is
  // 'soft'/'mixed'/'hard' — 'hard' DISCARDS uncommitted changes; the caller
  // gates this behind a strong inline confirm BEFORE calling this (this
  // action itself fires the request unconditionally). Same reply pattern as
  // `gitCherryPick`.
  gitReset: (sha: string, mode: 'soft' | 'mixed' | 'hard') => void
  // Branch-switcher / graph context menu "Merge into current branch" (G5c) —
  // may conflict, same reasoning as `gitCherryPick`. `ref` is a branch name or
  // a sha.
  gitMerge: (ref: string) => void
  // Rebase onto `upstream` (G5c/G6) — a branch name or a sha. `branch`
  // (G6 GitKraken-style drag-to-rebase — a dragged branch chip dropped onto a
  // commit/ref) checks out + rebases THAT branch instead of the current one;
  // omitted rebases the current branch (unchanged G5c behaviour). May
  // conflict, same reasoning as `gitCherryPick`.
  gitRebase: (upstream: string, branch?: string) => void
  // The conflict banner's Abort button (G5c): `kind` is `git.inProgress`
  // verbatim ('merge'/'rebase'/'cherry-pick'/'revert'). Same reply pattern as
  // `gitCherryPick`.
  gitOpAbort: (kind: string) => void
  // The conflict banner's Continue button (G5c). Same `kind` values and reply
  // pattern as `gitOpAbort` — git refuses (surfacing an error toast) if
  // conflicts remain.
  gitOpContinue: (kind: string) => void
  // Settings "SSH Keys" section: re-fetch the vault's key list. Fired on the
  // section opening/re-activating.
  refreshKeys: () => void
  // Generate a fresh passphrase-less ed25519 keypair. The reply lands as a
  // one-shot KeyOp push (toasted on failure) followed by a fresh KeyList push
  // that refreshes `keys` — this action never mutates `keys` optimistically.
  keyGenerate: (name: string, comment: string) => void
  // Import an existing pasted private key under `name`. Same reply pattern as
  // keyGenerate.
  keyImport: (name: string, privateKey: string) => void
  // Reveal a keypair's public (`private: false`) or private (`private: true`)
  // half. The reply lands as a one-shot KeyReveal push into `keyReveal`.
  keyReveal: (name: string, priv: boolean) => void
  // Dismiss the currently-shown reveal box (local-only — no wire request).
  clearKeyReveal: () => void
  // Delete a keypair (both halves, best-effort). Same reply pattern as
  // keyGenerate.
  keyDelete: (name: string) => void
  // Open (or focus) a read-only STREAM tab for a sub-agent (`kind:'subagent'`) or bash
  // job (`kind:'bash'`) by its numeric id: find-or-create (dedup by the stable
  // `sa:`/`bash:` id), activate it, and sync the stream view so the host starts streaming
  // THAT target's transcript / output tail.
  openStreamTab: (kind: 'subagent' | 'bash', targetId: number, title: string) => void
  // Stream-view chokepoint: derive {subagent, bash} from the CURRENTLY-ACTIVE tab (a
  // stream tab → its target; anything else → both null) and send SetStreamView, so
  // exactly ONE stream view is ever active (the active stream tab, else none). Called
  // from openStreamTab / activateTab / closeTab / session-switch (the four paths that
  // can change which tab is active).
  syncStreamView: () => void
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
  // Mark a session id "dying" right after firing its KillSession ('kill') or
  // DeleteSession ('delete') req (ResumePalette/StartScreen confirm).
  // Idempotent — marking the same id+kind twice (or a race) never duplicates
  // the entry.
  markDying: (id: string, kind: 'kill' | 'delete') => void
  // Kill-the-ATTACHED-session fast path: KillSession on the foreground session
  // sends the host straight to the swapper WITHOUT ever emitting a Snapshot
  // (only Hub pushes follow), so `session.id` would otherwise stay stale
  // forever and IndexPage would keep rendering the dead chat. Call this right
  // after firing that KillSession req to reset the session slice to
  // `initialSession` locally (hub/dyingSessions untouched — the follow-up Hub
  // push still needs to land to move the row into History) and clear any
  // per-session UI state that would otherwise render stale (tabs back to just
  // chat, active tab back to 'chat', any stuck switching overlay), mirroring
  // the Snapshot handler's `switched` branch. IndexPage's `sessionId === null`
  // gate then falls back to StartScreen immediately instead of waiting on a
  // push that isn't coming.
  detachSession: () => void
  // Agent-tab saving lifecycle tracker — set right before a SetAgent request,
  // cleared on the matching confirmatory push (AgentsValues success with the
  // new name visible, or AgentOp error). `seq` monotonically increases per
  // save so a stale reply cannot clear a newer request. `null` when no save
  // is in flight.
  agentSaving: {
    tabId: string
    seq: number
    originalName: string | null
    newName: string
  } | null
  // Set agentSaving to track a pending save (called right before the SetAgent
  // request is sent). `tabId` is the agent tab's client-local id, `originalName`
  // is the pre-edit name (null for create), `newName` is the trimmed final name.
  // Returns the assigned seq so the caller can pass it in the GuiReq.
  setAgentSaving: (tabId: string, originalName: string | null, newName: string) => number
  // Clear agentSaving (called on confirmation — AgentsValues success that
  // includes the saved agent, or AgentOp error toast). Takes the expected
  // `seq` to guard against stale clears; an undefined or mismatched seq is
  // a no-op.
  clearAgentSaving: (seq?: number) => void
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
  loading: null,
  loadingDismissed: false,
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

const initialOAuth: OAuthSlice = {
  phase: 'idle',
  url: null,
  userCode: null,
  verificationUrl: null,
  error: null,
  conns: [],
  providers: [],
}

const initialGit: GitStatus = {
  root: null,
  branch: null,
  detached: false,
  ahead: null,
  behind: null,
  staged: [],
  unstaged: [],
  error: null,
  keyName: null,
  inProgress: null,
  conflicted: [],
}

const initialGraph: GraphSlice = {
  commits: [],
  head: null,
  hasMore: false,
  loading: false,
  loadMode: 'replace',
  pendingRefresh: false,
  selectedSha: null,
  detail: null,
  graphMode: 'rail',
}

const initialActivity: ActivitySlice = {
  commits: [],
  loading: false,
  error: null,
  path: null,
}

const initialKeys: KeyInfo[] = []

const initialModelList: ModelListEntry[] = []

const initialRouteList: KomaState['routeList'] = null

const initialPalette: PaletteColors = {
  bg: '#0b0e14',
  fg: '#c8d3f5',
  accent: '#39ff14',
  dim: '#adadad',
  panel: '#2b2f38',
  warn: '#ffb43c',
  success: '#00c853',
  info: '#50c8ff',
  error: '#ff3c3c',
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
  setVar('--koma-warn', palette?.warn)
  setVar('--koma-success', palette?.success)
  setVar('--koma-info', palette?.info)
  setVar('--koma-error', palette?.error)
}

// Basename of a path — a diff tab's title (TabBar disambiguates colliding
// basenames with a dim parent-dir suffix at render time).
function tabBaseName(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

// De-duplicate a commit list by sha, keeping the FIRST occurrence's order — the
// load-more append path can re-receive an overlapping commit (a page boundary,
// or a commit reachable from two refs under `--all`), and a stable dedupe keeps
// the layout deterministic + the virtualized row keys unique.
function dedupCommits(commits: GitCommitNode[]): GitCommitNode[] {
  const seen = new Set<string>()
  const out: GitCommitNode[] = []
  for (const c of commits) {
    if (seen.has(c.sha)) continue
    seen.add(c.sha)
    out.push(c)
  }
  return out
}

// Mints a stable, client-local id for a new agent editor tab — independent of
// agentId (see the Tab union's 'agent' member comment for why).
let agentTabSeq = 0
function mintAgentTabId(): string {
  agentTabSeq += 1
  return `agent-${agentTabSeq}`
}

export const useKoma = create<KomaState>((set, get) => ({
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,
  ui: initialUi,
  config: initialConfig,
  oauth: initialOAuth,
  modelList: initialModelList,
  routeList: initialRouteList,
  settingsValues: null,
  effortOptions: null,
  usagePreview: null,
  dyingSessions: [],
  agents: [],
  catalogueModels: [],
  catalogueProviders: [],
  availableTools: [],
  git: initialGit,
  graph: initialGraph,
  activity: initialActivity,
  remoteBusy: null,
  commitDraft: '',
  keys: initialKeys,
  keyRevealResult: null,
  branches: [],
  branchesLoading: false,
  repos: [],
  activeRepoRoot: null,
  stashes: [],
  agentSaving: null,

  push: (env) => {
    switch (env.k) {
      case 'Snapshot': {
        // A Snapshot whose session id differs from the current one is a session
        // SWITCH. Captured BEFORE the set so it's readable AFTER (to sync the stream
        // view). The set reuses it to drop the OLD session's in-flight stream/
        // reasoning (it belongs to the old session — don't let it bleed into the new
        // view until the next send clears it) + reset the editor tabs.
        const switched = env.session !== get().session.id
        set((s) => {
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
              // A genuine switch also drops any stale startup splash — it
              // described the OLD attach's warm-up, and must not bleed into
              // the new session's view (the host will push a fresh `Loading`
              // for the new session if it's cold too).
              ...(switched
                ? { tabs: [makeChatTab()], activeTabId: 'chat', loading: null, loadingDismissed: false }
                : {}),
            },
            // A genuine switch also drops the OLD session's git/graph/activity
            // slices — they're host-driven for the PREVIOUS repo/session and
            // must not bleed into the new one until each panel's own
            // session-keyed effect re-fetches. The graph view MODE
            // (rail/bubble) is a user preference, not session data, so it's
            // preserved across the reset.
            ...(switched
              ? {
                  git: initialGit,
                  activity: initialActivity,
                  graph: { ...initialGraph, graphMode: s.graph.graphMode },
                  repos: [],
                  activeRepoRoot: null,
                }
              : {}),
          }
        })
        applyPaletteVars(env.palette)
        // A genuine switch reset the tabs to just chat (above) → no stream tab is
        // active. Sync the now-empty stream view so the host stops streaming the OLD
        // session's sub-agent/bash target (the new session's daemon starts with none
        // anyway). Fired AFTER the set so it reads the reset tab state.
        if (switched) get().syncStreamView()
        // A genuine switch also invalidates `settingsValues` — it's the OLD
        // session's name/workdir/toggles/effort, never refreshed by the Snapshot
        // envelope itself. Re-fetch so the (always-visible) EffortPicker trigger
        // pill and the Settings tab (if open) rehydrate for the NEW session
        // instead of showing the old one's values until Settings is reopened.
        if (switched) get().req({ r: 'GetSettings' })
        if (switched) get().refreshRepos()
        break
      }
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
                  toast: {
                    id: seq,
                    text: env.toast as string,
                    // Pass a recognised severity straight through (future-proofs
                    // "warn"/"success" if the host ever emits them); anything else
                    // (today: everything but "error") falls back to "info".
                    kind:
                      env.toastKind === 'error' || env.toastKind === 'warn' || env.toastKind === 'success'
                        ? env.toastKind
                        : 'info',
                  },
                }
              : s.ui,
          }
        })
        break
      case 'Hub':
        set((s) => {
          // Prune "dying" marks the moment a fresh Hub push confirms the
          // matching disposition landed. Kind-scoped: a killed session stays
          // on disk and MIGRATES from cooking to history on this very push —
          // so a 'kill' mark clears when the id drops out of COOKING
          // (regardless of it now appearing in history), and a 'delete' mark
          // clears when the id drops out of HISTORY. An id-agnostic
          // "absent from both lists" rule would keep a migrated-in history
          // row stuck spinning forever (the real bug this fixes).
          const cookingIds = new Set<string>(
            env.cooking.map((c) => c.id).filter((id): id is string => !!id),
          )
          const historyIds = new Set<string>(env.history.map((h) => h.id))
          return {
            hub: { ...s.hub, state: env.state, cooking: env.cooking, history: env.history },
            dyingSessions: s.dyingSessions.filter((d) =>
              d.kind === 'kill' ? cookingIds.has(d.id) : historyIds.has(d.id),
            ),
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
          }
        })
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
                        origin: env.origin ?? 'git',
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
            effort: env.effort ?? '',
          },
        }))
        break
      case 'EffortOptions':
        set(() => ({
          effortOptions: {
            options: env.options,
            selected: env.selected,
            note: env.note,
            state: env.state,
          },
        }))
        break
      case 'Loading':
        set((s) => ({
          ui: {
            ...s.ui,
            loading: env.active
              ? { active: env.active, workspace: env.workspace, awareness: env.awareness }
              : null,
          },
        }))
        break
      case 'AgentsValues': {
        // Normalize the wire's snake_case nested structs into the store's
        // camelCase shapes (see the AgentEntry/CatalogueModelEntry/
        // CatalogueProviderEntry comments — the envelope's OWN fields are
        // already camelCase, only the per-item fields need mapping).
        const agents: AgentEntry[] = env.agents.map((a) => ({
          name: a.name,
          description: a.description,
          conditions: a.conditions,
          source: a.source === 'global' || a.source === 'builtin' ? a.source : 'session',
          modelUuid: a.model_uuid,
          model: a.model,
          tools: a.tools,
          prompt: a.prompt,
        }))
        const catalogueModels: CatalogueModelEntry[] = env.catalogueModels.map((m) => ({
          uuid: m.uuid,
          name: m.name,
          modelId: m.model_id,
          providerUuid: m.provider_uuid,
        }))
        const catalogueProviders: CatalogueProviderEntry[] = env.catalogueProviders.map((p) => ({
          uuid: p.uuid,
          name: p.name,
          endpoint: p.endpoint,
        }))
        const availableTools = env.availableTools ?? []
        set((s) => {
          const liveNames = new Set(agents.map((a) => a.name))
          // A deleted agent's editor tab has nothing left to show — close it
          // automatically, derived from its absence in this fresh list (no
          // explicit "delete succeeded" ack exists on the wire). An
          // in-progress CREATE tab (agentId === null) is never touched here —
          // it isn't "in" the list yet by definition.
          let tabs = s.ui.tabs
          let activeTabId = s.ui.activeTabId
          const staleIds = new Set(
            tabs
              .filter((t) => t.kind === 'agent' && t.agentId !== null && !liveNames.has(t.agentId))
              .map((t) => t.id),
          )
          if (staleIds.size > 0) {
            tabs = tabs.filter((t) => !staleIds.has(t.id))
            // Multiple tabs could go stale from one push — land on chat
            // rather than compute per-removal left-neighbours (closeTab's
            // approach only makes sense for a single removal).
            if (staleIds.has(activeTabId)) activeTabId = 'chat'
          }
          return { agents, catalogueModels, catalogueProviders, availableTools, ui: { ...s.ui, tabs, activeTabId } }
        })
        // After updating agents, check if a pending agent save was confirmed
        // by the new list. This avoids premature rename/rebind: the tab's
        // `agentId` stays unchanged until the authoritative AgentsValues push
        // confirms the save landed. The seq check prevents a stale reply from
        // clearing a newer save request.
        const saving = get().agentSaving
        // reqSeq MUST match the current agentSaving.seq or this is a stale
        // reply. reqSeq === 0 (uncorrelated fallback from a read-only fetch
        // or host-built reply) never clears a pending save — the proper
        // SetAgent/DeleteAgent path always carries the real seq.
        if (saving && env.reqSeq === saving.seq) {
          const saved = get().agents.find((a) => a.name === saving.newName)
          if (saved) {
            get().clearAgentSaving(saving.seq)
            // Rename the agent tab so re-clicking its row in the sidebar
            // focuses this same tab instead of opening a duplicate.
            get().renameAgentTab(saving.originalName, saving.newName)
            // Toast a SUCCESS confirmation — the daemon's save succeeded.
            const text = saving.originalName && saving.originalName !== saving.newName
              ? `renamed to ${saving.newName}`
              : 'agent saved'
            const toastSeq = get().ui.toastSeq + 1
            set((s) => ({
              ui: { ...s.ui, toastSeq, toast: { id: toastSeq, text, kind: 'success' } },
            }))
          }
        }
        break
      }
      case 'OAuthState': {
        const KNOWN_PHASES: OAuthPhase[] = [
          'idle',
          'starting',
          'waiting_url',
          'waiting_code',
          'paste',
          'success',
          'failed',
        ]
        const phase: OAuthPhase = KNOWN_PHASES.includes(env.phase as OAuthPhase)
          ? (env.phase as OAuthPhase)
          : 'idle'
        set(() => ({
          oauth: {
            phase,
            url: env.url,
            userCode: env.userCode,
            verificationUrl: env.verificationUrl,
            error: env.error,
            // Normalize the wire's snake_case `account_id` to camelCase,
            // matching the AgentsValues normalization pattern.
            conns: env.conns.map((c) => ({
              uuid: c.uuid,
              name: c.name,
              provider: c.provider,
              email: c.email,
              plan: c.plan,
              accountId: c.account_id,
            })),
            providers: env.providers.map((p) => ({ id: p.id, label: p.label, kind: p.kind })),
          },
        }))
        break
      }
      case 'UsagePreview':
        set((s) => {
          // Drop a reply for a scope the user has since switched away from (a
          // rapid all/session toggle racing an in-flight request) — leave
          // `usagePreview` as-is (likely null, showing the loading row) until
          // the reply matching the CURRENT scope lands.
          if (env.scope !== s.ui.usageScope) return s
          // Drop a "session"-scope reply whose echoed session id no longer
          // matches the CURRENTLY attached session — the foreground session
          // switched while this request was in flight (scope stayed
          // "session" throughout), so this reply describes the OLD session
          // and must not render under the new attach.
          if (env.scope === 'session' && env.sessionId !== s.session.id) return s
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
      case 'GitStatus':
        set(() => ({
          git: {
            root: env.root,
            branch: env.branch,
            detached: env.detached,
            ahead: env.ahead,
            behind: env.behind,
            staged: env.staged,
            unstaged: env.unstaged,
            error: env.error,
            keyName: env.keyName,
            inProgress: env.inProgress,
            conflicted: env.conflicted,
          },
        }))
        break
      case 'GitDiff':
        set((s) => {
          const id = `gitdiff:${env.staged ? 'staged' : 'unstaged'}:${env.path}`
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
                        // GitDiff is always an actual git diff (never a
                        // non-git "virtual git" baseline) — unlike FileDiff,
                        // this reply has no `origin` field at all.
                        origin: 'git',
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'GitGraph':
        set((s) => {
          // Append (load-more) concatenates onto the existing page and dedupes;
          // replace (refresh / first load) drops the old page entirely. Only one
          // GitGraph request is ever in flight (see refreshGraph/loadMoreGraph),
          // so `loadMode` here is unambiguously the mode of THIS reply.
          const commits =
            s.graph.loadMode === 'append'
              ? dedupCommits([...s.graph.commits, ...env.commits])
              : env.commits
          return {
            graph: { ...s.graph, commits, head: env.head, hasMore: env.hasMore, loading: false },
          }
        })
        // A refreshGraph() that landed while this request was in flight couldn't
        // fire (serialization guard) and instead set pendingRefresh — now that
        // loading is committed false above, replay it.
        if (get().graph.pendingRefresh) {
          set((s) => ({ graph: { ...s.graph, pendingRefresh: false } }))
          get().refreshGraph()
        }
        break
      case 'CommitDetail':
        set((s) => {
          // Drop a stale reply for a since-changed selection (the echoed `sha` no
          // longer matches what's selected) — the detail pane keeps its loading
          // state until the reply matching the CURRENT selection lands.
          if (env.sha !== s.graph.selectedSha) return s
          return {
            graph: {
              ...s.graph,
              detail: {
                sha: env.sha,
                author: env.author,
                email: env.email,
                date: env.date,
                subject: env.subject,
                body: env.body,
                parents: env.parents,
                files: env.files,
                error: env.error,
              },
            },
          }
        })
        break
      case 'CommitDiff':
        set((s) => {
          const id = `commitdiff:${env.sha}:${env.path}`
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
                        // A commit diff is always a real git diff (`git show
                        // <sha>^1:…` vs `<sha>:…`) — never a "virtual git"
                        // baseline, so origin is always 'git'.
                        origin: 'git',
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'GitOp': {
        // Fetch/pull/push are the only ops that ever set `remoteBusy`; clearing
        // it unconditionally on every OTHER op is a harmless no-op (already null).
        const isRemote = env.op === 'fetch' || env.op === 'pull' || env.op === 'push'
        // Every op that can move HEAD or change the in-progress/conflict state
        // (branch-switcher/graph context menu ops G4 + the destructive/
        // interactive ops G5c: cherry-pick/revert/reset/merge/rebase, and the
        // conflict banner's abort/continue). All of these get a success toast
        // too — unlike the silent local mutations (stage/unstage/discard/
        // commit) — since HEAD/the branch list/conflict state just changed and
        // nothing else in the UI necessarily reflects that on its own.
        const HEAD_MOVING_OPS: Record<string, string> = {
          checkout: 'switched branch',
          createBranch: 'branch created',
          cherryPick: 'commit cherry-picked',
          revert: 'commit reverted',
          reset: 'branch reset',
          merge: 'merge complete',
          rebase: 'rebase complete',
          abort: 'operation aborted',
          continue: 'operation continued',
        }
        const isHeadMovingOp = env.op in HEAD_MOVING_OPS
        // Stash push/pop (GK4c) — neither moves HEAD, so they're kept OUT of
        // HEAD_MOVING_OPS (no graph refresh below), but still get a success
        // toast same as a head-moving op (nothing else in the UI reflects
        // "stashed"/"popped" on its own).
        const STASH_OPS: Record<string, string> = {
          stash: 'changes stashed',
          stashPop: 'stash applied',
        }
        const isStashOp = env.op in STASH_OPS
        // Surface a failed mutation the same de-duped way the Status case raises
        // a toast: only start a NEW toast when the text actually differs from
        // what's already showing, so a repeated failure (e.g. clicking "Stage
        // All" twice on a locked index) doesn't reset the auto-dismiss timer. A
        // SUCCESSFUL remote/head-moving op ALSO gets a toast — a short
        // confirmation using the host's own `message` if it sent one, else a
        // generic per-op label — so the outcome is visible even when nothing
        // else in the UI changes (e.g. a fetch with nothing new, or a
        // Continue that lands cleanly with no further conflicts). Local
        // mutations (stage/unstage/discard/commit) stay silent on success,
        // unchanged.
        const text = env.error
          ? `git ${env.op}: ${env.error}`
          : isRemote
            ? (env.message ?? `${env.op} complete`)
            : isHeadMovingOp
              ? HEAD_MOVING_OPS[env.op]
              : isStashOp
                ? STASH_OPS[env.op]
                : null
        const kind: 'error' | 'success' = env.error ? 'error' : 'success'
        set((s) => {
          const raise = !!text && text !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return {
            ui: raise
              ? { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind } }
              : s.ui,
            // A successful commit empties the draft, ready for the next
            // message. Any other op — or a failed commit — leaves it alone
            // (a failed commit's typed message must not be lost).
            ...(env.op === 'commit' && env.ok ? { commitDraft: '' } : {}),
            // A remote op is no longer in flight once its reply lands, success
            // OR failure — clear the sync toolbar's busy flag.
            ...(isRemote ? { remoteBusy: null } : {}),
          }
        })
        // A successful HEAD-moving op (checkout/createBranch/cherryPick/revert/
        // reset/merge/rebase/abort/continue) moved HEAD, the branch list, and/or
        // the in-progress/conflict state — refresh the commit graph (its HEAD
        // ring) so it isn't left stale. NO explicit GitStatus refresh here: the
        // host ALREADY auto-follows every one of these ops with its own fresh
        // GitStatus push right after the mutation (git_host.rs's
        // spawn_{checkout,create_branch,cherry_pick,revert,reset,merge,rebase,
        // op_abort,op_continue}_attached each `push_git_op` then recompute +
        // send a fresh `GitStatusResult` unconditionally) — firing a SECOND
        // `GitStatus` request here would just be a redundant full-tree
        // `git status` scan racing the host's own, doubling status-panel
        // flicker/load with no benefit. The graph, unlike status, is NEVER
        // auto-pushed by the host, so it still needs this explicit refresh. A
        // conflicting cherry-pick/merge/rebase/etc. returns `ok:false` (git's
        // conflict exit IS reported as a failure here), so this gate skips the
        // graph refresh for it — that's fine, HEAD hasn't finalized on a
        // conflict, so there's nothing new for the graph to reflect until
        // `continue` succeeds. The conflict banner still appears regardless,
        // because the host pushes a fresh GitStatus unconditionally after
        // every op, independent of this `ok` gate.
        if (isHeadMovingOp && env.ok) {
          get().refreshGraph()
        }
        // A stash push/pop changed the stash list either way (a failed pop
        // left it unchanged, but re-fetching is harmless) — refresh the
        // toolbar's Stash/Pop count. These ops never move HEAD, so — unlike
        // the branch above — this never triggers a graph refresh.
        if (isStashOp) {
          get().refreshStashes()
        }
        // A successful commit, fetch, pull, or push changes graph-visible refs
        // (new commits on HEAD, updated remote-tracking refs, or both) — refresh
        // the commit graph so it isn't left stale. Uses the existing serialized
        // refreshGraph() which guards against duplicate in-flight requests.
        // These ops are separate from HEAD-moving ops (checkout/merge/rebase etc.)
        // which are handled above, and from stash ops (no graph impact).
        if (env.ok && (env.op === 'commit' || env.op === 'fetch' || env.op === 'pull' || env.op === 'push')) {
          get().refreshGraph()
        }
        break
      }
      case 'KeyList':
        set(() => ({ keys: env.keys }))
        break
      case 'BranchList':
        set(() => ({ branches: env.branches, branchesLoading: false }))
        break
      case 'RepoList':
        set(() => ({ repos: env.repos, activeRepoRoot: env.active }))
        break
      case 'StashList':
        set(() => ({ stashes: env.entries }))
        break
      case 'Activity':
        set((s) => {
          // Drop a stale reply for a since-changed path filter (the echoed `path`
          // no longer matches what's currently requested) — lock-acquisition
          // order between two racing GitActivity requests isn't FIFO, so a
          // slower earlier reply can otherwise land after a faster later one and
          // clobber it. `null === null` still matches the whole-branch case.
          if (env.path !== s.activity.path) return s
          return {
            activity: { ...s.activity, commits: env.commits, error: env.error, loading: false },
          }
        })
        break
      case 'KeyReveal':
        set((s) => {
          // Toast only a COPY-public-key failure (`private: false`) — a
          // private-reveal failure already renders inline in the reveal box
          // (see SshKeysSettings' `revealedPrivate.error` branch) and would
          // otherwise double-surface. Same de-duped toast idiom as the KeyOp
          // case: only start a NEW toast when the text actually differs from
          // what's already showing.
          const text = env.error && !env.private ? `ssh key reveal: ${env.error}` : null
          const raise = !!text && text !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return {
            keyRevealResult: {
              name: env.name,
              private: env.private,
              content: env.content,
              error: env.error,
            },
            ui: raise
              ? { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind: 'error' } }
              : s.ui,
          }
        })
        break
      case 'KeyOp':
        set((s) => {
          // Same de-duped toast idiom as the GitOp case above: only start a NEW
          // toast when the text actually differs from what's already showing.
          const text = env.error ? `ssh key ${env.op}: ${env.error}` : null
          const raise = !!text && text !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return raise
            ? { ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind: 'error' } } }
            : {}
        })
        break
      case 'AgentOp': {
        // Daemon SetAgent/DeleteAgent result — surface the error as a toast
        // and clear the pending saving state. Success is authoritative via
        // AgentsValues, so only failures use this envelope.
        // reqSeq MUST match the current agentSaving.seq or this is a stale
        // reply (a prior request that landed after a newer one was issued).
        // reqSeq === 0 (uncorrelated fallback) never clears a pending save
        // — the new DaemonEvent::AgentOp from requests_agents.rs always
        // carries the proper seq, so only that path can clear.
        const saving = get().agentSaving
        if (!saving || env.reqSeq !== saving.seq) break
        if (!env.ok && env.error) {
          const text = `agent: ${env.error}`
          set((s) => {
            const raise = text !== s.ui.toast?.text
            const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
            return raise
              ? { ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } }, agentSaving: null }
              : { agentSaving: null }
          })
        } else {
          set(() => ({ agentSaving: null }))
        }
        break
      }
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
  dismissLoading: () => set((s) => ({ ui: { ...s.ui, loadingDismissed: true } })),
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
  openHelpTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'help')
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: 'help', kind: 'help' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'help' } }
    })
  },
  openAgentTab: (agentId) => {
    set((s) => {
      // Dedupe by agentId (NOT id — see the Tab union comment): re-clicking
      // the same agent's row, or "+ Add agent" while a blank create tab is
      // already open, focuses that existing tab instead of opening another.
      const existingTab = s.ui.tabs.find((t) => t.kind === 'agent' && t.agentId === agentId)
      if (existingTab) return { ui: { ...s.ui, activeTabId: existingTab.id } }
      const id = mintAgentTabId()
      const tabs: Tab[] = [...s.ui.tabs, { id, kind: 'agent', agentId }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
  },
  renameAgentTab: (oldAgentId, newAgentId) => {
    if (oldAgentId === newAgentId) return
    // Only `agentId` changes — `id` (and thus the tab's React key/identity
    // and activeTabId) stays exactly as it was, so the open AgentTab instance
    // is never remounted by this rebind.
    set((s) => ({
      ui: {
        ...s.ui,
        tabs: s.ui.tabs.map((t) =>
          t.kind === 'agent' && t.agentId === oldAgentId ? { ...t, agentId: newAgentId } : t,
        ),
      },
    }))
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
  refreshGitStatus: () => {
    get().req({ r: 'GitStatus' })
  },
  openGitDiffTab: (path, staged) => {
    const id = `gitdiff:${staged ? 'staged' : 'unstaged'}:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const title = `${tabBaseName(path)}${staged ? ' (staged)' : ''}`
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) => (t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t))
        : [...s.ui.tabs, { id, kind: 'diff', path, title, loading: true, staged }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'GitDiff', path, staged })
  },
  openGraphTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'graph')
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: 'graph', kind: 'graph' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'graph' } }
    })
    // No wire fetch here — the GraphTab fires refreshGraph on mount.
  },
  refreshGraph: () => {
    // Serialize: at most one GitGraph request in flight, ever. If a load-more
    // (or another refresh) is already in flight, defer instead of racing it —
    // the 'GitGraph' reducer replays this once that reply lands and clears
    // `loading`. This keeps `loadMode` unambiguous for whichever reply comes
    // back next.
    if (get().graph.loading) {
      set((s) => ({ graph: { ...s.graph, pendingRefresh: true } }))
      return
    }
    set((s) => ({
      graph: { ...s.graph, loading: true, loadMode: 'replace', pendingRefresh: false },
    }))
    get().req({ r: 'GitGraph', limit: 200, skip: 0 })
  },
  loadMoreGraph: () => {
    const g = get().graph
    // Guard against a duplicate in-flight load and a no-op past the last page.
    if (g.loading || !g.hasMore) return
    set((s) => ({ graph: { ...s.graph, loading: true, loadMode: 'append' } }))
    get().req({ r: 'GitGraph', limit: 200, skip: g.commits.length })
  },
  selectCommit: (sha) => {
    set((s) => ({
      graph: {
        ...s.graph,
        selectedSha: sha,
        // Clear stale detail when the selection actually changes so the pane
        // shows a loading state instead of the previous commit's detail.
        detail: s.graph.selectedSha === sha ? s.graph.detail : null,
      },
    }))
    get().req({ r: 'GitCommitDetail', sha })
  },
  clearSelection: () => set((s) => ({ graph: { ...s.graph, selectedSha: null, detail: null } })),
  setGraphMode: (mode) => {
    set((s) => ({ graph: { ...s.graph, graphMode: mode } }))
  },
  refreshActivity: (path) => {
    const p = path ?? null
    set((s) => ({ activity: { ...s.activity, loading: true, path: p } }))
    get().req({ r: 'GitActivity', path: p, limit: 800 })
  },
  openCommitDiffTab: (sha, path) => {
    const id = `commitdiff:${sha}:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const title = `${tabBaseName(path)} @ ${sha.slice(0, 7)}`
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) => (t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t))
        : [...s.ui.tabs, { id, kind: 'diff', path, title, loading: true, commitSha: sha }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'GitCommitDiff', sha, path })
  },
  setCommitDraft: (text) => set(() => ({ commitDraft: text })),
  gitStage: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitStage', paths })
  },
  gitUnstage: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitUnstage', paths })
  },
  gitDiscard: (paths) => {
    if (paths.length === 0) return
    get().req({ r: 'GitDiscard', paths })
  },
  gitCommit: (message) => {
    if (!message.trim()) return
    get().req({ r: 'GitCommit', message })
  },
  setGitKey: (name) => {
    get().req({ r: 'SetGitKey', name })
  },
  gitFetch: () => {
    set(() => ({ remoteBusy: 'fetch' }))
    get().req({ r: 'GitFetch' })
  },
  gitPull: () => {
    set(() => ({ remoteBusy: 'pull' }))
    get().req({ r: 'GitPull' })
  },
  gitPush: () => {
    set(() => ({ remoteBusy: 'push' }))
    get().req({ r: 'GitPush' })
  },
  refreshBranches: () => {
    set(() => ({ branchesLoading: true }))
    get().req({ r: 'GitBranchList' })
  },
  refreshRepos: () => {
    get().req({ r: 'GitRepos' })
  },
  setActiveRepo: (root) => {
    set((s) => ({
      activeRepoRoot: root,
      graph: { ...initialGraph, graphMode: s.graph.graphMode },
      activity: initialActivity,
    }))
    get().req({ r: 'SetActiveRepo', root })
  },
  gitStash: () => {
    get().req({ r: 'GitStash' })
  },
  gitStashPop: () => {
    get().req({ r: 'GitStashPop' })
  },
  refreshStashes: () => {
    get().req({ r: 'GitStashList' })
  },
  gitCheckout: (ref) => {
    if (!ref.trim()) return
    get().req({ r: 'GitCheckout', ref })
  },
  gitCreateBranch: (name, start, checkout) => {
    if (!name.trim()) return
    get().req({ r: 'GitCreateBranch', name: name.trim(), start, checkout })
  },
  gitCherryPick: (sha) => {
    get().req({ r: 'GitCherryPick', sha })
  },
  gitRevert: (sha) => {
    get().req({ r: 'GitRevert', sha })
  },
  gitReset: (sha, mode) => {
    get().req({ r: 'GitReset', sha, mode })
  },
  gitMerge: (ref) => {
    if (!ref.trim()) return
    get().req({ r: 'GitMerge', ref })
  },
  gitRebase: (upstream, branch) => {
    if (!upstream.trim()) return
    get().req({ r: 'GitRebase', upstream, branch: branch ?? null })
  },
  gitOpAbort: (kind) => {
    get().req({ r: 'GitOpAbort', kind })
  },
  gitOpContinue: (kind) => {
    get().req({ r: 'GitOpContinue', kind })
  },
  refreshKeys: () => {
    get().req({ r: 'KeyList' })
  },
  keyGenerate: (name, comment) => {
    if (!name.trim()) return
    get().req({ r: 'KeyGenerate', name: name.trim(), comment })
  },
  keyImport: (name, privateKey) => {
    if (!name.trim() || !privateKey.trim()) return
    get().req({ r: 'KeyImport', name: name.trim(), privateKey })
  },
  keyReveal: (name, priv) => {
    get().req({ r: 'KeyReveal', name, private: priv })
  },
  clearKeyReveal: () => set(() => ({ keyRevealResult: null })),
  keyDelete: (name) => {
    get().req({ r: 'KeyDelete', name })
  },
  openStreamTab: (kind, targetId, title) => {
    const id = kind === 'subagent' ? `sa:${targetId}` : `bash:${targetId}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tab: Tab =
        kind === 'subagent'
          ? { id, kind: 'subagent', agentId: targetId, title }
          : { id, kind: 'bash', jobId: targetId, title }
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, tab]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    // This stream tab is now active → tell the host to stream its target's content.
    get().syncStreamView()
  },
  syncStreamView: () => {
    const { tabs, activeTabId } = get().ui
    const tab = tabs.find((t) => t.id === activeTabId)
    const subagent = tab && tab.kind === 'subagent' ? tab.agentId : null
    const bash = tab && tab.kind === 'bash' ? tab.jobId : null
    // Pin the ids to the current session — they're per-session counters daemon-side, so
    // the daemon needs the session to disambiguate (agent 0 / bash 1 exist in every session).
    get().req({ r: 'SetStreamView', subagent, bash, session: get().session.id })
  },
  closeTab: (id) => {
    if (id === 'chat') return
    set((s) => {
      const idx = s.ui.tabs.findIndex((t) => t.id === id)
      if (idx < 0) return s
      const tabs = s.ui.tabs.filter((t) => t.id !== id)
      // If the closed tab was active, fall back to the left neighbour. idx-1 is
      // always valid (tabs[0] is the chat tab), so this never underflows.
      const activeTabId =
        s.ui.activeTabId === id ? s.ui.tabs[idx - 1]?.id ?? 'chat' : s.ui.activeTabId
      return { ui: { ...s.ui, tabs, activeTabId } }
    })
    // The active tab may have changed (closed the active one) — re-sync the stream
    // view so the host stops streaming a just-closed stream tab's target (or starts
    // streaming the neighbour if focus fell onto another stream tab).
    get().syncStreamView()
  },
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
    // A GIT-panel diff tab (has `staged`) re-requests via GitDiff, echoing
    // the SAME staged/unstaged side it was opened for; a plain File-changed
    // diff tab (no `staged`) re-requests via FileDiff — the two paths are
    // NOT interchangeable (different host handlers, different tab-id scheme).
    if (isDiff && tab.kind === 'diff') {
      if (tab.commitSha !== undefined) {
        // A commit-graph diff tab re-requests via GitCommitDiff — checked FIRST
        // (it carries no `staged`, so it would otherwise wrongly fall into the
        // FileDiff branch and fetch a working-tree diff for a historical path).
        get().req({ r: 'GitCommitDiff', sha: tab.commitSha, path: tab.path })
      } else if (tab.staged !== undefined) {
        get().req({ r: 'GitDiff', path: tab.path, staged: tab.staged })
      } else {
        get().req({ r: 'FileDiff', path: tab.path })
      }
    }
    // Re-focusing the Settings tab re-requests its values so they're fresh (the
    // name/workdir may have changed via other paths, e.g. the RenameOverlay).
    // Also re-fetch the SSH Keys vault list — the Settings tab stays mounted
    // (CSS-hidden) across a close/reopen, so without this the "SSH Keys"
    // section would only ever reflect whatever the vault looked like on the
    // FIRST open of the session.
    if (tab.kind === 'settings') {
      get().req({ r: 'GetSettings' })
      get().refreshKeys()
    }
    // Sync the stream view to the now-active tab: a stream tab → stream its target;
    // any other tab (chat/diff/settings) → clear the view. The host/daemon dedupe an
    // unchanged view, so activating a non-stream tab repeatedly is cheap.
    get().syncStreamView()
  },
  focusPlanSection: () => set((s) => ({ ui: { ...s.ui, focusPlanTick: s.ui.focusPlanTick + 1 } })),
  setUsageScope: (scope) => set((s) => ({ ui: { ...s.ui, usageScope: scope } })),
  markDying: (id, kind) =>
    set((s) =>
      s.dyingSessions.some((d) => d.id === id && d.kind === kind)
        ? s
        : { dyingSessions: [...s.dyingSessions, { id, kind }] },
    ),
  detachSession: () => {
    set((s) => ({
      // Fresh object (not spread from the old session) — nothing about the
      // just-killed session is worth preserving, mirrors initialSession's
      // shape exactly.
      session: { ...initialSession },
      ui: {
        ...s.ui,
        tabs: [makeChatTab()],
        activeTabId: 'chat',
        // Defensive: clear a stuck switching overlay too, in case one was
        // mid-flight (only Snapshot/Hub normally clear it, neither of which
        // is guaranteed to arrive promptly on a self-kill).
        switchingTo: null,
        // Defensive: also drop any stale startup splash — it described the
        // now-dead session's warm-up and must not linger over StartScreen.
        loading: null,
        loadingDismissed: false,
      },
    }))
    // Tabs just reset to chat-only → no stream tab is active; tell the host
    // to stop streaming whatever the dead session's stream tab was targeting
    // (mirrors the Snapshot handler's `switched` branch).
    get().syncStreamView()
  },
  // Track a pending agent-save request so the AgentTab can show a spinner,
  // disable the Save button, and receive success/error feedback without
  // premature rename/rebind. `seq` monotonically increases per save so the
  // confirming push (AgentsValues or AgentOp) can reject a stale reply by
  // comparing against `get().agentSaving.seq`.
  setAgentSaving: (tabId, originalName, newName) => {
    const current = get().agentSaving
    const seq = (current?.seq ?? 0) + 1
    set(() => ({ agentSaving: { tabId, seq, originalName, newName } }))
    return seq
  },
  clearAgentSaving: (seq) => {
    set((s) => {
      if (seq !== undefined && s.agentSaving?.seq !== seq) return {} // stale, don't clear
      return { agentSaving: null }
    })
  },
}))
