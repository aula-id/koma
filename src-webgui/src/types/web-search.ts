export type SearchProvider = 'built_in' | 'firecrawl' | 'tavily' | 'exa'
export function searchChip(provider: SearchProvider | null | undefined): string {
  switch (provider) {
    case 'tavily':
      return 'tavily'
    case 'firecrawl':
      return 'firecrawl'
    case 'exa':
      return 'exa'
    default:
      return 'default'
  }
}
export type SearchStatus = { provider: SearchProvider; saved_keys: SearchProvider[] }
export type SearchReply = { req_seq: number; status: SearchStatus; error: string | null }
export const SEARCH_PROVIDERS: { id: SearchProvider; label: string; website: string }[] = [
  { id: 'built_in', label: 'Built-in DuckDuckGo', website: 'https://duckduckgo.com' },
  { id: 'firecrawl', label: 'Firecrawl', website: 'https://www.firecrawl.dev' },
  { id: 'tavily', label: 'Tavily', website: 'https://www.tavily.com' },
  { id: 'exa', label: 'Exa', website: 'https://exa.ai' },
]
export function canEnable(status: SearchStatus, provider: SearchProvider, draft: string): boolean {
  return provider === 'built_in' || !!draft.trim() || status.saved_keys.includes(provider)
}
export function activation(status: SearchStatus, provider: SearchProvider, draft: string) {
  const enabled = status.provider === provider
  if (enabled) return provider === 'built_in' ? null : { provider: 'built_in' as SearchProvider, key: null }
  if (!canEnable(status, provider, draft)) return null
  return { provider, key: provider !== 'built_in' && draft.trim() ? draft.trim() : null }
}

export type SearchSettingsState = {
  status: SearchStatus | null
  drafts: Partial<Record<SearchProvider, string>>
  pending: { seq: number; provider: SearchProvider } | null
  loadSeq: number | null
  error: string | null
}
export const initialSearchSettings: SearchSettingsState = { status: null, drafts: {}, pending: null, loadSeq: null, error: null }
export type SearchSettingsAction =
  | { type: 'load'; seq: number }
  | { type: 'edit'; provider: SearchProvider; key: string }
  | { type: 'begin'; provider: SearchProvider; seq: number }
  | { type: 'reply'; reply: SearchReply }
export function searchSettingsReducer(state: SearchSettingsState, action: SearchSettingsAction): SearchSettingsState {
  switch (action.type) {
    case 'load': return { ...state, loadSeq: action.seq }
    case 'edit':
      if (state.pending || state.status?.provider === action.provider) return state
      return { ...state, drafts: { ...state.drafts, [action.provider]: action.key } }
    case 'begin':
      if (state.pending || !state.status || !activation(state.status, action.provider, state.drafts[action.provider] ?? '')) return state
      return { ...state, pending: { seq: action.seq, provider: action.provider }, error: null }
    case 'reply': {
      const { reply } = action
      if (reply.req_seq === state.pending?.seq) {
        return { ...state, status: reply.error ? state.status : reply.status, pending: null, error: reply.error,
          drafts: reply.error ? state.drafts : { ...state.drafts, [state.pending.provider]: '' } }
      }
      if (reply.req_seq === state.loadSeq) return { ...state, status: reply.status, error: reply.error, loadSeq: null }
      return state
    }
  }
}
