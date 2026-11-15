export const METHODS = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve', 'configure', 'detach'] as const;
/** The ten v1 methods. hello `capabilities` always lists exactly these. */
export type V1Method = typeof METHODS[number];
export const CONFIG_METHODS = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign', 'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set'] as const;
export type ConfigMethod = typeof CONFIG_METHODS[number];
export type Method = V1Method | ConfigMethod | 'sessions.info';
/** Unknown values of an open enum-like string are kept verbatim; treat them as "other". */
export type Open<T extends string> = T | (string & {});
export interface Policy { plan: 'allow' | 'forbid'; approval: 'default' | 'reject' }
export type Attachment = { kind: 'image_file'; path: string } | { kind: 'pasted_text'; text: string; name?: string | null };
export interface Input { text: string; attachments?: Attachment[] }
export interface CreateOptions { cwd: string; model_id?: string; policy?: Policy; external_approvals?: boolean }
export type ModelRoleName = 'main' | 'awareness' | 'safeguard' | 'compactor' | 'planner';
export interface ProviderInput { name: string; endpoint: string; api_type: 'openai' | 'anthropic'; api_key?: string }
export interface ModelAssignment { provider: string; model_id: string; route?: string }
export interface ConfigureOptions { providers?: ProviderInput[]; models?: Partial<Record<ModelRoleName, ModelAssignment>> }
export interface ConfigureResult {
  providers: { name: string; uuid: string }[];
  models: Partial<Record<ModelRoleName, { uuid: string; model_id: string; provider: string }>>;
}
export interface ResumeOptions { session_id: string; policy?: Policy; external_approvals?: boolean }
export type Turn = { id: string; state: 'running'; error: string | null } | { id: string; state: 'finished' | 'interrupted' | 'failed'; error: string | null };
export interface State {
  revision: number; external_approvals: boolean; pending_approval: { call_id: string; reason: string | null } | null; gate: { kind: "tool" | "plan" | "mission" | "unknown"; reason: string | null; supported: boolean } | null;
  session_id: string; cwd: string; model_id: string; policy: Policy; turn: Turn | null;
  working: boolean; awaiting_approval: boolean; approval_reason: string | null;
  pending_count: number; messages: { role: 'system' | 'user' | 'assistant' | 'tool'; content: string }[];
  streaming: string | null;
}
export interface Reason { code: string; message: string }
export type Rejected = { disposition: 'rejected'; turn_id: string | null; reason: Reason };
export type SubmitResult = { disposition: 'started'; turn_id: string; reason: null } | Rejected;
export type EnqueueResult = { disposition: 'queued'; turn_id: null; reason: null } | Rejected;
export type InterruptResult = { disposition: 'interrupted'; turn_id: string; reason: null } | Rejected;
export type ReplacementResult = SubmitResult & { interrupted_turn_id: string | null };
export type StateEvent = { type: 'event'; event: 'state'; seq: number; state: State; resynced: boolean };
export type OutputAppendEvent = { type: 'event'; event: 'output_append'; seq: number; revision: number; offset: number; text: string };
export type TurnEvent = { type: 'event'; event: 'turn'; seq: number; revision: number; turn: Turn | null };
export type Event = StateEvent | OutputAppendEvent | TurnEvent;
export interface HelloLimits {
  frame_bytes?: number; request_ids?: number; text_bytes?: number;
  attachments?: { count?: number; pasted_bytes?: number; total_bytes?: number };
  config?: { lock_wait_ms?: number };
  time?: { ipc_ms?: number };
  [key: string]: unknown;
}
/** `methods`, `features`, `events` are empty and `limits` is `{}` for an older adapter that does not send them. */
export interface Hello {
  type: 'hello'; protocol: 1; binary: { path: string; version: string }; diagnostics: string[]; capabilities: string[];
  methods: string[]; features: string[]; events: string[]; limits: HelloLimits;
}
export interface RequestOptions { signal?: AbortSignal; timeoutMs?: number }
export interface ClientOptions {
  /** Absolute path to the Koma executable. Always launched as [executable, 'sdk', 'serve'], never through a shell. */
  executable?: string;
  /** Test/custom adapter argument override; default sdk serve. Never interpreted by a shell. */
  adapterArgs?: readonly string[];
  /** Required at handshake; defaults to every v1 method. */
  requiredCapabilities?: readonly V1Method[];
  timeoutMs?: number;
  /** Inherited environment plus these overrides. Useful for isolated configuration. */
  env?: NodeJS.ProcessEnv;
}
export type Delivery = 'not_sent' | 'unknown';
export type Binding = Open<'unbound' | 'coding' | 'room_live' | 'room_closed'>;
/** Optional keys an adapter with feature `errors.reason` may add to an error. */
export interface ErrorDetails { reason?: string; binding?: Binding; allowed?: Binding[]; written?: boolean; detail?: Record<string, unknown> }
export class SdkError extends Error {
  /** Open string from the reason registry; branch on it, never on the message. */
  declare reason?: string;
  /** On binding errors: the adapter's current binding and the bindings where the call is allowed. */
  declare binding?: Binding;
  declare allowed?: Binding[];
  /** True when an `unknown` outcome is known to have committed bytes. */
  declare written?: boolean;
  declare detail?: Record<string, unknown>;
  constructor(public readonly code: string, message: string, public readonly delivery: Delivery = 'not_sent', public readonly requestId?: string, public readonly method?: Method) {
    super(message); this.name = 'SdkError';
  }
}
/** A dispatched mutation may have taken effect. Never automatically retry. */
export class DeliveryUnknownError extends SdkError {
  sessionId?: string;
  turnId?: string;
  constructor(code: string, message: string, requestId: string, method: Method) {
    super(code, message, 'unknown', requestId, method); this.name = 'DeliveryUnknownError';
  }
}
export class TurnReplacedError extends SdkError {
  constructor(public readonly turnId: string, public readonly observedTurnId: string) {
    super('turn_replaced', `Turn ${turnId} was replaced by ${observedTurnId}; its final outcome is not available`); this.name = 'TurnReplacedError';
  }
}

export type PublicRole = ModelRoleName;
export type ConfigViewName = 'global' | 'session';
export interface Provider {
  provider_uuid: string; name: string; api_type: string; endpoint: string; endpoint_redacted: boolean;
  managed: boolean; has_key: boolean; key_state: Open<'set' | 'missing' | 'unreadable'>; model_count: number;
}
export interface OAuthConn { oauth_uuid: string; name: string; provider: string; has_token: boolean; expires_at: number }
export interface Model {
  model_uuid: string; name: string; model_id: string; provider_uuid: string;
  provider_kind: Open<'provider' | 'oauth' | 'missing'>; provider_name: string | null; route: string | null;
  roles: PublicRole[]; other_roles: boolean; source_uuid: string | null;
}
export interface RoleRoute {
  role: PublicRole; origin: Open<'session' | 'global' | 'inherit_main' | 'legacy' | 'default'>;
  model_uuid: string | null; source_uuid: string | null; configured_model: string | null;
  provider_uuid: string | null; provider_name: string | null; effective_model: string | null;
  reason: Open<'unsupported_route' | 'credentials_unavailable' | 'provider_missing' | 'reference_deleted' | 'unassigned'> | null;
}
export interface ConfigView {
  revision: string; file: Open<'ok' | 'missing'>; view: Open<ConfigViewName>;
  providers: Provider[]; providers_truncated: boolean; oauth: OAuthConn[]; oauth_truncated: boolean;
  models: Model[]; models_truncated: boolean; roles: RoleRoute[];
  main_fallback: Open<'unconfigured' | 'provider_removed' | 'no_key' | 'not_signed_in'> | null;
}
export interface Propagation {
  bound?: Open<'reloaded' | 'failed' | 'not_bound' | 'applied_by_daemon'>;
  broadcast?: Open<'sent' | 'failed' | 'sent_by_daemon'>;
  rebind?: Open<'done' | 'failed' | 'not_needed' | 'done_by_daemon'>;
  rooms?: Open<'next_start'>;
  [key: string]: string | undefined;
}
export interface Limits { max_output_tokens: number; context_window_limit: number; context_model_alias: string; effective_window: number | null }
export interface SessionUsage {
  last_prompt_tokens: number; last_cached_tokens: number; output_tokens_total: number; cost_total: number;
  context: { prompt_tokens: number; effective_window: number; estimated: boolean; cached_tokens: number | null; drss_active: boolean } | null;
}
/** Per-session facts beside `state`. Newer means a higher `observed_at_ms`, never a higher state revision. */
export interface SessionInfo {
  name: string; effort: string; routes: RoleRoute[]; limits: Limits; usage: SessionUsage | null;
  daemon: { version: string; ipc_protocol: number; capabilities: string[] };
  observed_at_ms: number;
}
export interface ConfigGetParams { view?: ConfigViewName }
export interface ProviderDeleteParams { provider_uuid: string; if_revision?: string; dry_run?: boolean }
export interface ModelDeleteParams { model_uuid: string; if_revision?: string; dry_run?: boolean }
export interface RoleUnassignParams { role: PublicRole; scope?: ConfigViewName; if_revision?: string }
export interface EffortGetParams { wait_ms?: number }
export interface EffortSetParams { effort: string; if_main_model?: string | null }
export interface LimitsSetParams { max_output_tokens?: number; context_window_limit?: number; context_model_alias?: string }
/** Coding-path mutation results also carry the bound session's `session_info`. */
export interface MutationResult { config: ConfigView; propagation: Propagation; session_info?: SessionInfo }
export interface ProviderDeleteResult extends MutationResult { dry_run: boolean; removed: { provider_uuid: string; models_removed: string[]; main_reset: boolean } }
export interface ModelDeleteResult extends MutationResult { dry_run: boolean; removed: { model_uuid: string; roles_released: PublicRole[]; main_reset: boolean } }
export interface RoleUnassignResult extends MutationResult { changed: boolean; previous_model_uuid: string | null }
export interface EffortOptions { state: Open<'ready' | 'loading' | 'unsupported'>; options: string[]; selected: string; note: string | null; main_model_uuid: string | null }
export interface EffortResult { effort: string; main_model_uuid: string | null; superseded: boolean; session_info?: SessionInfo }
export type LimitsResult = Limits & { session_info?: SessionInfo };
