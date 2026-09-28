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

