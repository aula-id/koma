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
// `isKomaFree` flags the auto-provisioned keyless koma-free provider
// (`ApiType::KomaFree`, render.rs `PushProvider`). It's not a user-editable
// provider — the Connector PROVIDERS list hides it (the free tier is
// dropdown-only). Optional-tolerant: absent on a host build that doesn't
// project the flag yet (and on every real provider).
export type Provider = { id: string; name: string; endpoint: string; hasKey: boolean; isKomaFree?: boolean }

// OAuth types (`OAuthConn`/`OAuthProviderEntry`) live in store/koma.ts now —
// they're populated by the real `OAuthState` push envelope, not a local stub.

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
  // Host-projected flag for the advertised koma-free keyless tier — pinned to
  // the TOP of the session-main quick-picker. Optional-tolerant: absent on a
  // host build that doesn't advertise a free model.
  free?: boolean
  // For a `local`-scope override CLONED from a global entry, the uuid of that
  // global (daemon `ModelEntry.source_uuid`, wire `sourceUuid`). The ModelPicker
  // matches its active session-Main against the global rows by THIS exact id
  // (identity), not by name. Optional-tolerant: absent on every global row, the
  // synthetic free row, a directly-authored local entry, AND on an override created
  // before the field existed (the picker then falls back to a name match).
  sourceUuid?: string
}

// Live per-provider model-id catalogue entry (replaces ModelForm's
// DEMO_MODEL_IDS). Fed by GuiReq ListModels -> PushEnvelope ModelList.
export type ModelListEntry = string

// Live per-model ROUTE (OpenRouter upstream endpoint) entry — real provider
// names + per-token pricing + recent uptime for the chosen model. Replaces
// ModelForm's DEMO_ROUTES. Fed by GuiReq ListRoutes -> PushEnvelope RouteList.
// Mirrors the host's ModelEndpointSnapshot (render.rs `rename_all = "camelCase"`):
// `pricePrompt`/`priceCompletion` are USD-per-token strings ("0" = free),
// `uptimeLast30m` is a 0-100 percentage. All value fields optional-tolerant.
export type RouteEntry = {
  // The upstream endpoint/route id the daemon stores on the model (`SetModel.route`).
  // Falls back to `providerName` when the host omits an explicit id.
  name?: string
  providerName: string
  pricePrompt?: string
  priceCompletion?: string
  uptimeLast30m?: number
}
