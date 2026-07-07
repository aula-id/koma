import { create } from 'zustand'

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

export type PushEnvelope =
  | {
      k: 'Snapshot'
      session: string
      state: string
      messages: ChatMessage[]
      title: string
      palette: PaletteColors
    }
  | { k: 'StreamMsg'; session: string; text: string }
  | { k: 'Reasoning'; session: string; text: string }
  | { k: 'Status'; session: string; working: boolean; toast: string | null }
  | { k: 'Hub'; state: string; cooking: HubCookingEntry[]; history: HubHistoryEntry[] }

// JS -> Rust request payloads. (Promoted to a global ambient type in
// koma.d.ts alongside the rest of the window bridge contract in a later
// step; kept local for now since there are no consumers yet.)
export type GuiReq =
  | { r: 'Ready' }
  | { r: 'Submit'; text: string }
  | { r: 'SelectSession'; id: string }
  | { r: 'NewSession' }

// ---- Store shape --------------------------------------------------------

type SessionSlice = {
  id: string | null
  state: string | null
  messages: ChatMessage[]
  title: string
  working: boolean
  stream: string
  reasoning: string
}

type HubSlice = {
  state: string | null
  cooking: HubCookingEntry[]
  history: HubHistoryEntry[]
}

type KomaState = {
  session: SessionSlice
  hub: HubSlice
  palette: PaletteColors
  // Rust -> JS: apply an authoritative push envelope. Always REPLACES the
  // relevant slice fields — never accumulates/appends.
  push: (env: PushEnvelope) => void
  // JS -> Rust: typed request helper, tags the envelope { t: 'req', ...g }.
  req: (g: GuiReq) => void
}

const initialSession: SessionSlice = {
  id: null,
  state: null,
  messages: [],
  title: '',
  working: false,
  stream: '',
  reasoning: '',
}

const initialHub: HubSlice = {
  state: null,
  cooking: [],
  history: [],
}

const initialPalette: PaletteColors = {
  bg: '#0b0e14',
  fg: '#c8d3f5',
}

export const useKoma = create<KomaState>((set) => ({
  session: initialSession,
  hub: initialHub,
  palette: initialPalette,

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
          },
          palette: env.palette,
        }))
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
    }
  },

  req: (g) => {
    try {
      window.ipc?.postMessage(JSON.stringify({ t: 'req', ...g }))
    } catch {
      /* ipc unavailable */
    }
  },
}))
