// Shared config-panel types (MCP servers, providers, models). Mirrors the
// daemon's `AppConfig` projection (providers/models/mcp_servers) — see
// `Config` push envelope in `store/koma.ts`. Kept in one module so the panel
// components (McpPanel/ConnectorPanel + their sub-views), the store, and
// koma.d.ts's GuiReq union all agree on shape.

export type Transport = 'stdio' | 'http'

// Daemon's `McpServerEntry.args`/`env` are `Vec<String>`/`Vec<(String,String)>`;
// the GUI form fields are single strings (`args`: space-separated,
// `env`: "KEY=VAL, KEY2=VAL2") — the gui bridge on the Rust side parses/joins
// these (mirrors `mcp/state.rs` parse_args/parse_env/join_env).
export type McpServer = {
  id: string
  name: string
  enabled: boolean
  transport: Transport
  command: string
  args: string
  env: string
  url: string
}

// `hasKey` is a presence flag, NOT the key itself — the daemon never sends the
// plaintext API key to the webview (devtools are enabled; it'd be DOM/console
// readable). ProviderForm's key input always starts empty on edit; saving with
// it blank keeps the existing stored key (see `upsert_provider` daemon-side).
export type Provider = { id: string; name: string; endpoint: string; hasKey: boolean }

export type OAuthProv = 'OpenAI' | 'Kilo Code' | 'Anthropic'
export type OAuthConn = { id: string; provider: OAuthProv; account: string }

export type Scope = 'global' | 'local'
export type Role = 'main' | 'awareness' | 'safeguard' | 'compactor' | 'planner'

export type Model = {
  id: string
  name: string
  modelId: string
  provider: string
  route: string
  roles: Role[]
  scope: Scope
}

// Live per-provider model-id catalogue entry (replaces ModelForm's
// DEMO_MODEL_IDS). Fed by GuiReq ListModels -> PushEnvelope ModelList.
export type ModelListEntry = string
