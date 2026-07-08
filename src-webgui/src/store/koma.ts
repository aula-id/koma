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
  | { k: 'Status'; session: string; working: boolean; toast: string | null; toastKind?: string }
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
    }
  // Reply to GuiReq ListModels — live per-provider model-id catalogue. Field
  // is `models` to match the daemon's PushEnvelope::ModelList { provider, models }.
  | { k: 'ModelList'; provider: string; models: ModelListEntry[] }
  // Reply to GuiReq ListRoutes — live per-model OpenRouter endpoint list. Echoes
  // the provider+modelId it was fetched for so ModelForm can discard a stale
  // reply that no longer matches its current selection. Empty `routes` = a
  // non-OpenRouter provider (UI shows only the synthetic "Auto" row).
  | { k: 'RouteList'; provider: string; modelId: string; routes: RouteEntry[] }

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
  attachments: [],
  searchResults: [],
  mode: 'auto',
  pendingSteer: [],
  awaitingApproval: false,
  approvalReason: null,
  pendingCall: null,
}

const initialHub: HubSlice = {
  state: null,
  cooking: [],
  history: [],
}

const initialUi: UiSlice = {
  omnisearchOpen: false,
  composerInsert: null,
  composerRefill: null,
  scrollTick: 0,
  switchingTo: null,
  toast: null,
  toastSeq: 0,
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

export const useKoma = create<KomaState>((set) => ({
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,
  ui: initialUi,
  config: initialConfig,
  modelList: initialModelList,
  routeList: initialRouteList,

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
            // flight) has landed — clear the loader.
            ui: { ...s.ui, switchingTo: null },
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
            session: { ...s.session, working: env.working },
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
  requestScrollBottom: () => set((s) => ({ ui: { ...s.ui, scrollTick: s.ui.scrollTick + 1 } })),
  startSwitching: (name) => set((s) => ({ ui: { ...s.ui, switchingTo: name } })),
  cancelSwitching: () => set((s) => ({ ui: { ...s.ui, switchingTo: null } })),
  dismissToast: (id) =>
    set((s) => (s.ui.toast?.id === id ? { ui: { ...s.ui, toast: null } } : s)),
}))
