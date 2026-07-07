import { create } from 'zustand'
import type { McpServer, Provider, Model, ModelListEntry } from '../types/config'

// ---- Bridge contract types (Rust -> JS push envelopes) ----------------

export type ChatMessage = {
  role: 'user' | 'assistant'
  content: string
  reasoning: string | null
}

export type PaletteColors = {
  bg: string
  fg: string
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
  name: string
  status: 'running' | 'done' | 'killed' | 'error'
  summary: string
}

export type BashJobEntry = {
  id: string
  cmd: string
  status: 'running' | 'done' | 'killed' | 'error'
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
      attachments: AttachmentEntry[]
    }
  | { k: 'StreamMsg'; session: string; text: string }
  | { k: 'Reasoning'; session: string; text: string }
  | { k: 'Status'; session: string; working: boolean; toast: string | null }
  | { k: 'Hub'; state: string; cooking: HubCookingEntry[]; history: HubHistoryEntry[] }
  | { k: 'SearchResults'; query: string; items: SearchResultEntry[] }
  // Authoritative config projection (mcp/providers/models) — global, not
  // per-session. REPLACES the whole config slice, pushed on config change and
  // on (re)attach.
  | { k: 'Config'; mcp: McpServer[]; providers: Provider[]; models: Model[] }
  // Reply to GuiReq ListModels — live per-provider model-id catalogue.
  | { k: 'ModelList'; provider: string; items: ModelListEntry[] }

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
  attachments: AttachmentEntry[]
  searchResults: SearchResultEntry[]
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
  attachments: [],
  searchResults: [],
}

const initialHub: HubSlice = {
  state: null,
  cooking: [],
  history: [],
}

const initialUi: UiSlice = {
  omnisearchOpen: false,
  composerInsert: null,
}

const initialConfig: ConfigSlice = {
  mcp: [],
  providers: [],
  models: [],
}

const initialModelList: ModelListEntry[] = []

const initialPalette: PaletteColors = {
  bg: '#0b0e14',
  fg: '#c8d3f5',
}

const HEX_RE = /^#[0-9a-fA-F]{6}$/

// Live palette sync: repaint the --koma-bg/--koma-fg CSS vars whenever a
// Snapshot lands with a palette (home of the glue that used to live in
// Terminal.tsx's OSC 5380 handler).
function applyPaletteVars(palette: PaletteColors) {
  if (typeof document === 'undefined') return
  if (palette?.bg && HEX_RE.test(palette.bg)) {
    document.documentElement.style.setProperty('--koma-bg', palette.bg)
  }
  if (palette?.fg && HEX_RE.test(palette.fg)) {
    document.documentElement.style.setProperty('--koma-fg', palette.fg)
  }
}

export const useKoma = create<KomaState>((set) => ({
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,
  ui: initialUi,
  config: initialConfig,
  modelList: initialModelList,

  push: (env) => {
    switch (env.k) {
      case 'Snapshot':
        set((s) => ({
          session: {
            ...s.session,
            id: env.session,
            state: env.state,
            messages: env.messages,
            title: env.title,
            subagents: env.subagents,
            bash: env.bash,
            // Defensive fallback: tolerates a host build that hasn't started
            // projecting attachments[] on the Snapshot envelope yet.
            attachments: env.attachments ?? [],
          },
          palette: env.palette,
        }))
        applyPaletteVars(env.palette)
        break
      case 'StreamMsg':
        set((s) => ({ session: { ...s.session, stream: env.text } }))
        break
      case 'Reasoning':
        set((s) => ({ session: { ...s.session, reasoning: env.text } }))
        break
      case 'Status':
        set((s) => ({ session: { ...s.session, working: env.working } }))
        break
      case 'Hub':
        set((s) => ({
          hub: { ...s.hub, state: env.state, cooking: env.cooking, history: env.history },
        }))
        break
      case 'SearchResults':
        set((s) => ({ session: { ...s.session, searchResults: env.items } }))
        break
      case 'Config':
        set(() => ({ config: { mcp: env.mcp, providers: env.providers, models: env.models } }))
        break
      case 'ModelList':
        set(() => ({ modelList: env.items }))
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
}))
