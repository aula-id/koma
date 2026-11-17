export const METHODS = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve', 'configure', 'detach'] as const;
/** The ten v1 methods. hello `capabilities` always lists exactly these. */
export type V1Method = typeof METHODS[number];
export const CONFIG_METHODS = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign', 'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set'] as const;
export type ConfigMethod = typeof CONFIG_METHODS[number];
export const SESSION_METHODS = ['sessions.info', 'sessions.list', 'sessions.rename', 'sessions.delete', 'sessions.stop', 'sessions.transcript'] as const;
export const USAGE_METHODS = ['usage.query'] as const;
export const SKILL_METHODS = ['skills.list', 'skills.get', 'skills.read_file', 'skills.install', 'skills.remove', 'skills.create', 'skills.update'] as const;
export const ROOM_METHODS = ['room.list', 'room.open', 'room.attach', 'room.close', 'room.send', 'room.interrupt', 'room.control', 'room.status', 'room.history', 'room.shelf.list', 'room.item', 'room.trajectory', 'room.reports', 'room.spend', 'room.team.get', 'room.models.set', 'room.avatars.upsert', 'room.avatars.remove'] as const;
export type Method = V1Method | ConfigMethod | typeof SESSION_METHODS[number] | typeof USAGE_METHODS[number] | typeof SKILL_METHODS[number] | typeof ROOM_METHODS[number];
/** Unknown values of an open enum-like string are kept verbatim; treat them as "other". */
export type Open<T extends string> = T | (string & {});
export interface Policy { plan: 'allow' | 'forbid'; approval: 'default' | 'reject' }
export type Attachment = { kind: 'image_file'; path: string } | { kind: 'pasted_text'; text: string; name?: string | null };
export interface Input { text: string; attachments?: Attachment[] }
/** `name` needs the adapter feature `create.name`; it is trimmed and must be 1 to 200 characters without control characters. */
export interface CreateOptions { cwd: string; model_id?: string; policy?: Policy; external_approvals?: boolean; name?: string }
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
/** Events of the ROOM binding are declared with the room types below (`RoomEvent`). */
export interface HelloLimits {
  frame_bytes?: number; request_ids?: number; text_bytes?: number;
  skills?: { inline_zip_bytes?: number; zip_bytes?: number };
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

/** A warning an adapter attaches to a result, for example `rename_failed` on `create`. */
export interface Warning { code: string; message: string }

export interface SessionRow {
  session_id: string; name: string | null; workdir: string | null; kind: Open<'coding' | 'room'>;
  created_at: number | null; updated_at: number | null; live: boolean; working: boolean | null; locked: boolean; bound: boolean;
  parent_session_id: string | null; room_session_id: string | null; room_live: boolean;
}
export interface SessionsListParams { kind?: 'coding' | 'room' | 'all'; live?: boolean; limit?: number; cursor?: string }
export interface SessionsListResult { sessions: SessionRow[]; next_cursor: string | null }
export interface SessionRenameParams { session_id: string; name: string }
export interface SessionRenameResult { session: SessionRow }
export interface SessionDeleteResult { deleted: true; kind: Open<'coding' | 'room'>; orphaned_room_session_id: string | null }
export interface SessionStopResult { stopped: true; was_live: boolean }
export interface SessionTranscriptParams { session_id: string; since?: number; limit?: number }
/** `id` only grows and is never reused. `truncated` is present (true) when `text` was cut at 262144 bytes. */
export interface TranscriptMessage { id: number; role: 'system' | 'user' | 'assistant' | 'tool'; text: string; created_at: number; truncated?: true }
/** Messages with an id above `since`, oldest first. Pass `next_since` back as `since` for the next page. */
export interface SessionTranscript { session_id: string; live: boolean; messages: TranscriptMessage[]; next_since: number | null }

export type UsageGroupBy = 'none' | 'model' | 'role' | 'session';
export type UsageBucketSize = 'none' | 'hour' | 'day' | 'week';
export interface UsageQueryParams {
  from_at?: number; to_at?: number; session_id?: string; workdir?: string; group_by?: UsageGroupBy; bucket?: UsageBucketSize;
  tz_offset_minutes?: number; limit?: number;
}
export interface UsageTotals { calls: number; tokens_in: number; tokens_cached: number; tokens_out: number; cost: number }
export interface UsageGroup extends UsageTotals {
  key: string; label: string | null; role_kind?: Open<'main' | 'sub'>; agent?: string | null; kind?: Open<'coding' | 'room'> | null; deleted?: boolean;
}
export interface UsageBucket extends UsageTotals { start_at: number }
export interface UsageResult {
  from_at: number; to_at: number; ledger: Open<'ok' | 'missing'>; totals: UsageTotals;
  groups?: UsageGroup[]; groups_truncated?: boolean; buckets?: UsageBucket[];
}

export type SkillScope = Open<'computer' | 'project' | 'external'>;
/** `extra-root:<n>` carries the configured root's index. */
export type SkillTier = Open<'global' | 'claude' | 'project-agent' | 'project-agents'>;
export interface SkillRow {
  name: string; scope: SkillScope; tier: SkillTier; description: string; triggers: string[]; triggers_truncated: boolean;
  tags: string[]; tags_truncated: boolean; pinned: boolean; editable: boolean; generation: string; path: string;
  effective: boolean; shadowed_by: SkillTier | null;
}
export interface SkillDetail extends SkillRow {
  instruction: string; declared_tools: string[]; declared_tools_truncated: boolean; companion_files: string[];
  companion_files_truncated: boolean; structured_save_supported: boolean; raw: string | null;
}
export interface SkillsListParams {
  scope?: 'computer' | 'project' | 'external' | 'all'; tier?: string; workdir?: string; include_diagnostics?: boolean; limit?: number; cursor?: string;
}
export interface SkillsListResult {
  project_workdir: string | null; skills: SkillRow[]; diagnostics: { path: string; error: string }[]; diagnostics_truncated: boolean; next_cursor: string | null;
}
export interface SkillAddress { tier?: string; workdir?: string; name: string }
export interface SkillGetParams extends SkillAddress { include_raw?: boolean }
export interface SkillReadFileParams extends SkillAddress { path: string; generation?: string }
export interface SkillFileResult { path: string; text: string; bytes: number }
export interface SkillCreateParams {
  scope: 'computer' | 'project'; workdir?: string; name: string; description: string; instruction: string; triggers?: string[]; allowed_tools?: string[];
}
export interface SkillPatch { name?: string; description?: string; triggers?: string[]; allowed_tools?: string[]; instruction?: string }
export interface SkillUpdateParams { tier: string; workdir?: string; name: string; generation?: string; patch: SkillPatch }
/** Exactly one of `data_b64` and `path` (an absolute file on the adapter host). */
export interface SkillInstallParams { scope: 'computer' | 'project'; workdir?: string; name?: string; data_b64?: string; path?: string }
export interface SkillRemoveParams { tier: string; workdir?: string; name: string; generation?: string }
export interface SkillWriteResult { skill: SkillDetail; propagation: Propagation }
export interface SkillUpdateResult extends SkillWriteResult { renamed: boolean }
export interface SkillRemoveResult { removed: true; now_effective: SkillRow | null; propagation: Propagation }

export type LineKind = Open<'chief' | 'lane_chat' | 'visible' | 'user' | 'shell' | 'reasoning' | 'tool_call' | 'tool_result' | 'other'>;
export interface RoomInfo {
  session_id: string; parent_session_id: string | null; parent_name: string | null; name: string | null; workdir: string | null;
  live: boolean; locked: boolean; bound: boolean; created_at: number | null; updated_at: number | null;
}
export interface RoomLine {
  line_seq: number; kind: LineKind; author: string; display: string | null; alias: string | null; lane: Open<'chat' | 'work'> | null;
  audience: Open<'user' | 'team'> | null; text: string | null; call_id: string | null; tool: string | null; args: string | null;
  ok: boolean | null; result: string | null; replay: boolean; origin: Open<'room' | 'adapter_echo'>; received_at_ms: number; truncated: boolean;
}
export interface LaneStatus {
  avatar: string; display: string; alias: string; role: string; lane: Open<'chat' | 'work'> | null;
  phase: Open<'running' | 'idle' | 'paused' | 'submitted' | 'done' | 'failed'>; task: string; started_at_ms: number;
  last_event_at_ms: number; steps: number; last_tool: string; error: string;
}
export interface TimelineEvent { at_ms: number; avatar: string; marker: string; label: string }
export interface RoomCounts { open: number; verified: number; returned: number; reports: number }
export interface RoomStatus {
  session_id: string; live: boolean; fresh: boolean; age_ms: number | null; now_at_ms: number; counts: RoomCounts;
  counts_basis: Open<'first_400_items' | 'disk_scan'>; paused: string[]; paused_truncated: boolean; lanes: LaneStatus[]; lanes_truncated: boolean;
  events: TimelineEvent[]; digest: string; timeline: string; trajectory_outline: string; shelf_fingerprint: string; truncated: boolean;
}
export interface AttachResult {
  room: RoomInfo; started: boolean; daemon: { version: string | null; features: string[]; template_revision: string | null };
  epoch: string; history: { first_line_seq: number | null; last_line_seq: number | null; count: number; replayed: number };
  tail: RoomLine[]; status: RoomStatus | null; roster: string[]; roster_truncated: boolean;
}
export interface RoomsListParams { parent_session_id?: string; live?: boolean; limit?: number; cursor?: string }
export interface RoomsListResult { rooms: RoomInfo[]; next_cursor: string | null }
export interface RoomOpenParams { parent_session_id: string; tail?: number; wait_ms?: number }
export interface RoomAttachParams { session_id: string; tail?: number; wait_ms?: number }
export interface RoomCloseParams { stop?: boolean }
export interface RoomCloseResult { closed: true; stopped: boolean }
/** `shell` sends the text as a `!` command and `to` addresses one avatar. `shell` cannot be combined with `to` or attachments. */
export interface RoomSendParams { text: string; to?: string; shell?: boolean; attachments?: Attachment[] }
export interface RoomSendResult { sent: true; acked: boolean; inserts: string[]; echo_line_seq: number }
export interface RoomControlParams { action: 'pause' | 'resume' | 'stop' | 'redirect'; avatar: string; note?: string }
/** `epoch` defaults to the room's current epoch. At most one of the two bounds. */
export interface RoomHistoryParams { epoch?: string; after_line_seq?: number; before_line_seq?: number; limit?: number; kinds?: string[]; author?: string }
export interface RoomHistoryResult {
  epoch: string; first_line_seq: number | null; last_line_seq: number | null; lines: RoomLine[]; has_more_before: boolean; has_more_after: boolean;
}
export interface ShelfListParams { prefix?: string; limit?: number; cursor?: string }
export interface ShelfRow { key: string; revision: number; updated_at: number; kind: string; title: string; status: string }
export interface ShelfItem extends ShelfRow { content: string; json: Record<string, unknown> | null; truncated: boolean }
export interface ShelfListResult { rows: ShelfRow[]; total: number; next_cursor: string | null; fingerprint: string }
export interface GoalParams { goal?: string }
export interface GoalNode {
  id: string; label: string; kind: string; status: string | null; severity: string | null; note: string | null;
  resource_key: string | null; revision: number | null; rank: number; row: number;
}
export interface Goal {
  goal_key: string; counts: RoomCounts; columns: number; rows: number; nodes: GoalNode[]; nodes_truncated: boolean;
  edges: { from: string; to: string; rel: string | null }[]; edges_truncated: boolean;
}
export interface TrajectoryResult { goals: Goal[]; goals_truncated: boolean }
export interface ReportsParams { include_markdown?: boolean; limit?: number; cursor?: string }
export interface Report {
  key: string; revision: number; updated_at: number; title: string; audience: string; severity_summary: string; go_no_go: string;
  finding_keys: string[]; finding_keys_truncated: boolean; markdown: string | null;
}
export interface ReportsResult { reports: Report[]; next_cursor: string | null }
export interface SpendEntry { calls: number; prompt_tokens: number; completion_tokens: number; cost: number }
export interface GoalSpend { goal_key: string; total: SpendEntry; by_avatar: (SpendEntry & { avatar: string })[]; by_avatar_truncated: boolean }
export interface SpendResult {
  goals: GoalSpend[]; goals_truncated: boolean;
  ledger: { ledger: Open<'ok' | 'missing'>; totals: UsageTotals; by_avatar: { avatar: string; totals: UsageTotals }[]; by_avatar_truncated: boolean };
}

export type TeamTarget = 'template' | 'room';
export type Origin = Open<'base' | 'template' | 'session' | 'role' | 'default'>;
export interface Field<T> { value: T | null; origin: Origin }
export interface LaneView { lane: Open<'chat' | 'work'>; provider_uuid: Field<string>; model: Field<string>; effort: Field<string>; model_label: string | null; resolves: boolean }
export interface AvatarView {
  id: string; role: string; defined_in: Open<'base' | 'template' | 'session'>; removable: boolean; display: Field<string>; display_effective: string;
  alias: Field<string>; alias_effective: string; lanes: LaneView[];
}
export interface RoleLaneView { provider_uuid: string | null; model: string | null; effort: string | null }
export interface RoleView { id: string; display: string; shelf_allow: string[]; skills_allow: string[] | null; chat: RoleLaneView; work: RoleLaneView; defined_in: Open<'base' | 'template' | 'session'> }
export interface ModelChoice { id: string; name: string; model_id: string; provider_name: string | null; provider_uuid: string | null }
export interface Team {
  target: Open<TeamTarget>; revision: string; layer_present: boolean; template_revision: string; template_pending_restart: boolean;
  avatars: AvatarView[]; avatars_truncated: boolean; roles: RoleView[]; roles_truncated: boolean;
  choices: ModelChoice[]; choices_truncated: boolean; choices_source: Open<'room_daemon' | 'config'>;
  warnings: { layer: Open<'template' | 'session'>; path: string; message: string }[]; warnings_truncated: boolean;
}
export interface TeamGetParams { target?: TeamTarget }
/** At least one of `model` and `effort`; `null` clears the lane unit in the target layer. */
export interface TeamModelParams { target: TeamTarget; avatar: string; lane: 'chat' | 'work'; model?: string | null; effort?: string | null; if_revision?: string }
export interface TeamAvatarUpsertParams { target: TeamTarget; id: string; role?: string; display?: string | null; alias?: string | null; if_revision?: string }
export interface TeamAvatarRemoveParams { target: TeamTarget; id: string; if_revision?: string }
export interface TeamWriteResult { team: Team; applies: Open<'live' | 'next_reload' | 'next_start'>; propagation: Propagation }
export interface TeamAvatarRemoveResult extends TeamWriteResult { reverted_to: Open<'base' | 'template'> | null }

/** Events of the ROOM binding. Every one carries the room's `epoch`. Lines and status the client recovers from `room.history` carry `recovered: true`. */
export type RoomLineEvent = { type: 'event'; event: 'room.line'; seq: number; epoch: string; line: RoomLine; recovered?: boolean };
export type RoomDeltaEvent = { type: 'event'; event: 'room.delta'; seq: number; epoch: string; author: string; lane: Open<'chat' | 'work'>; kind: Open<'text' | 'reasoning'>; text: string };
export type RoomStatusEvent = { type: 'event'; event: 'room.status'; seq: number; epoch: string; status: RoomStatus; recovered?: boolean };
export type RoomNoticeEvent = { type: 'event'; event: 'room.notice'; seq: number; epoch: string; code: Open<'room' | 'stalled' | 'recovered' | 'layer_warnings'>; text: string };
export type RoomClosedEvent = { type: 'event'; event: 'room.closed'; seq: number; epoch: string; session_id: string; reason: Open<'taken_over' | 'daemon_exit' | 'stream_error'> };
export type RoomGapEvent = { type: 'event'; event: 'room.gap'; seq: number; epoch: string; after_line_seq: number; dropped_lines: number; dropped_deltas: number };
export type RoomEvent = RoomLineEvent | RoomDeltaEvent | RoomStatusEvent | RoomNoticeEvent | RoomClosedEvent | RoomGapEvent;
export const ROOM_EVENTS = ['room.line', 'room.delta', 'room.status', 'room.notice', 'room.closed', 'room.gap'] as const;
