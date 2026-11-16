import { isAbsolute } from 'node:path';
import { Transport, isRoomEvent, type Incoming } from './transport.js';
import { configureParams, local, object, string, textId, validateInput, validatePolicy } from './validate.js';
import { DeliveryUnknownError, SdkError, TurnReplacedError, type ClientOptions, type ConfigureOptions, type ConfigureResult, type CreateOptions, type ResumeOptions, type RequestOptions, type State, type Hello, type Input, type SubmitResult, type EnqueueResult, type InterruptResult, type ReplacementResult, type Event, type Turn, type Method, type SessionInfo, type ConfigView, type ConfigGetParams, type ProviderDeleteParams, type ProviderDeleteResult, type ModelDeleteParams, type ModelDeleteResult, type RoleUnassignParams, type RoleUnassignResult, type EffortGetParams, type EffortOptions, type EffortSetParams, type EffortResult, type Limits, type LimitsSetParams, type LimitsResult, type HelloLimits, type Warning, type SessionsListParams, type SessionsListResult, type SessionRenameParams, type SessionRenameResult, type SessionDeleteResult, type SessionStopResult, type SessionTranscript, type SessionTranscriptParams, type UsageQueryParams, type UsageResult, type SkillsListParams, type SkillsListResult, type SkillGetParams, type SkillDetail, type SkillReadFileParams, type SkillFileResult, type SkillCreateParams, type SkillWriteResult, type SkillUpdateParams, type SkillUpdateResult, type SkillInstallParams, type SkillRemoveParams, type SkillRemoveResult, type RoomsListParams, type RoomsListResult, type RoomOpenParams, type RoomAttachParams, type AttachResult, type RoomCloseResult, type RoomSendParams, type RoomSendResult, type RoomControlParams, type RoomStatus, type RoomHistoryParams, type RoomHistoryResult, type ShelfListParams, type ShelfListResult, type ShelfItem, type GoalParams, type TrajectoryResult, type ReportsParams, type ReportsResult, type SpendResult, type Team, type TeamGetParams, type TeamModelParams, type TeamAvatarUpsertParams, type TeamAvatarRemoveParams, type TeamWriteResult, type TeamAvatarRemoveResult, type TeamTarget, type RoomEvent } from './types.js';
export * from './types.js';

/** A stateless adapter is replaced once this many requests were sent, well before the per-process cap. */
export const HOST_RECYCLE_AFTER = 60_000;

/** Largest inline skill zip (decoded) an adapter that advertises no limit accepts. */
export const INLINE_ZIP_BYTES = 29_360_128;
/** Longest base64 text `skills.install` accepts. */
export const INLINE_ZIP_BASE64_CHARS = 39_146_840;
const MAX_NAME_CHARS = 200;

/** Copies caller params so later edits by the caller cannot change what is validated against the reply. */
function plain(params: object | undefined, ids: readonly string[] = []): Record<string, unknown> {
  return local(() => {
    if (params !== undefined) object(params);
    const out = { ...params } as Record<string, unknown>;
    for (const key of ids) textId(out[key]);
    return out;
  });
}

/** The scope is always sent, so a call without one is global on every adapter. */
function unassignParams(params: RoleUnassignParams): Record<string, unknown> {
  const out = plain(params, ['role']);
  out.scope ??= 'global';
  return out;
}

/** A session name as the adapter checks it: no control characters (before trimming), then trimmed to 1 to 200 characters. */
function sessionName(name: unknown): string {
  return local(() => {
    string(name);
    if (/[\u0000-\u001f\u007f]/.test(name)) throw new Error('name must not contain control characters');
    const trimmed = name.trim();
    const length = [...trimmed].length;
    if (length < 1 || length > MAX_NAME_CHARS) throw new Error('name must be 1 to 200 characters');
    return trimmed;
  });
}

const SKILL_NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;
/** A new skill name: lowercase letters, digits and single hyphens, at most 64 bytes, not `skill` or `readme`. */
function newSkillName(name: unknown): string {
  if (typeof name !== 'string' || !SKILL_NAME.test(name) || Buffer.byteLength(name) > 64 || name === 'skill' || name === 'readme') {
    throw invalid('invalid_name', 'Skill name must be lowercase letters, digits and single hyphens, at most 64 bytes, and not skill or readme');
  }
  return name;
}

export class Client {
  private connections = new Set<Transport>();
  private closed = false;
  constructor(private readonly options: ClientOptions = {}) {
    if (options.timeoutMs !== undefined && (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0)) throw new SdkError('invalid_input', 'timeoutMs must be positive');
  }
  private async connect(): Promise<Transport> {
    if (this.closed) throw new SdkError('closed', 'Client is closed');
    const t = new Transport(this.options); this.connections.add(t);
    try { await t.ready; return t; }
    catch (e) { await this.release(t); throw e; }
  }
  async release(t: Transport): Promise<void> { this.connections.delete(t); await t.close(); }
  async probe(): Promise<Hello> { const t = await this.connect(); try { return t.hello!; } finally { await this.release(t); } }
  /** A handle for calls that need no session. It starts its adapter on first use and replaces it before the request-id limit. */
  host(): Host { return new Host(() => this.connect(), t => this.release(t)); }
  /** Write providers and model roles before a session exists. No daemon is started. */
  async configure(options: ConfigureOptions, request?: RequestOptions): Promise<ConfigureResult> {
    const params = configureParams(options);
    const transport = await this.connect();
    try { return await transport.request<ConfigureResult>('configure', params, request); }
    finally { await this.release(transport); }
  }
  async create(options: CreateOptions, request?: RequestOptions): Promise<Session> {
    textId(options.cwd);
    if (!isAbsolute(options.cwd)) throw new SdkError('invalid_input', 'cwd must be absolute on daemon host');
    const policy = options.policy ?? { plan: 'forbid', approval: 'reject' };
    local(() => validatePolicy(policy));
    if (policy.approval === 'default' && !options.external_approvals) throw new SdkError('invalid_input', 'approval default requires external_approvals');
    const params: Record<string, unknown> = { ...options, policy };
    if (options.name !== undefined) params.name = sessionName(options.name);
    return this.bind('create', params, request, options.name === undefined ? undefined : t => {
      if (!t.hello?.features.includes('create.name')) throw new SdkError('unsupported_feature', 'Adapter does not support create.name');
    });
  }
  async resume(options: ResumeOptions, request?: RequestOptions): Promise<Session> {
    textId(options.session_id);
    if (options.policy) {
      local(() => validatePolicy(options.policy));
      if (options.policy.approval === 'default' && !options.external_approvals) throw new SdkError('invalid_input', 'approval default requires external_approvals');
    }
    return this.bind('resume', { ...options }, request);
  }
  /** Start the room of a coding session (when it is not running) and take its live view. See `Room`. */
  async roomOpen(params: RoomOpenParams, request?: RequestOptions): Promise<Room> {
    return this.bindRoom('room.open', plain(params, ['parent_session_id']), params.wait_ms ?? 12_000, request);
  }
  /** Take the live view of a running room. Any other viewer of that room loses its stream. */
  async roomAttach(params: RoomAttachParams, request?: RequestOptions): Promise<Room> {
    return this.bindRoom('room.attach', plain(params, ['session_id']), params.wait_ms ?? 2_000, request);
  }
  private async bindRoom(method: 'room.open' | 'room.attach', params: Record<string, unknown>, waitMs: number, request?: RequestOptions): Promise<Room> {
    const timeoutMs = Math.max(this.options.timeoutMs ?? 30_000, waitMs + 5_000);
    return this.attach<AttachResult, Room>(method, params, { timeoutMs, ...request }, (t, result) => new Room(this, t, result));
  }
  private bind(method: 'create' | 'resume', params: Record<string, unknown>, options?: RequestOptions, check?: (t: Transport) => void): Promise<Session> {
    return this.attach<{ state: State; session_info?: SessionInfo; warnings?: Warning[] }, Session>(method, params, options,
      (t, result) => new Session(this, t, result.state, result.session_info ?? null, result.warnings ?? []), check);
  }
  /** Start an adapter, send the call that binds it and wrap the result; the adapter is closed when any of that fails. */
  private async attach<T, R>(method: Method, params: Record<string, unknown>, options: RequestOptions | undefined, make: (t: Transport, result: T) => R, check?: (t: Transport) => void): Promise<R> {
    const t = await this.connect();
    try {
      check?.(t);
      return make(t, await t.request<T>(method, params, options));
    } catch (e) { await this.release(t); throw e; }
  }
  async close(): Promise<void> {
    this.closed = true;
    await Promise.all([...this.connections].map(t => this.release(t)));
  }
  async [Symbol.asyncDispose](): Promise<void> { await this.close(); }
}

/**
 * Calls that work in every binding of an adapter: sessions, usage, skills, rooms on disk and the room template.
 * `Host`, `Session` and `Room` share them.
 */
abstract class HostCalls {
  protected abstract request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T>;
  protected abstract helloLimits(): Promise<HelloLimits>;
  /** Registry rows of coding sessions and rooms, newest first, one page per call. */
  async listSessions(params?: SessionsListParams, request?: RequestOptions): Promise<SessionsListResult> {
    return this.request('sessions.list', plain(params), request);
  }
  /** Refused for another live coding session (`reason` "live"). */
  async renameSession(params: SessionRenameParams, request?: RequestOptions): Promise<SessionRenameResult> {
    const out = plain(params, ['session_id']);
    try { out.name = sessionName(out.name); } catch (e) { throw invalid('invalid_name', e instanceof Error ? e.message : String(e)); }
    return this.request('sessions.rename', out, request);
  }
  async deleteSession(params: { session_id: string }, request?: RequestOptions): Promise<SessionDeleteResult> {
    return this.request('sessions.delete', plain(params, ['session_id']), request);
  }
  /** Stops the session's daemon, which disconnects every client of it. */
  async stopSession(params: { session_id: string }, request?: RequestOptions): Promise<SessionStopResult> {
    return this.request('sessions.stop', plain(params, ['session_id']), request);
  }
  /**
   * The stored messages of a coding session, read from disk: no daemon is started, so a session that is not live
   * reads the same as a running one. Secrets are scrubbed. A room is refused (`out_of_range`): use `history`.
   */
  async readTranscript(params: SessionTranscriptParams, request?: RequestOptions): Promise<SessionTranscript> {
    return this.request('sessions.transcript', plain(params, ['session_id']), request);
  }
  async queryUsage(params?: UsageQueryParams, request?: RequestOptions): Promise<UsageResult> {
    return this.request('usage.query', plain(params), request);
  }
  async listSkills(params?: SkillsListParams, request?: RequestOptions): Promise<SkillsListResult> {
    return this.request('skills.list', plain(params), request);
  }
  async getSkill(params: SkillGetParams, request?: RequestOptions): Promise<SkillDetail> {
    return this.request('skills.get', plain(params, ['name']), request);
  }
  async readSkillFile(params: SkillReadFileParams, request?: RequestOptions): Promise<SkillFileResult> {
    return this.request('skills.read_file', plain(params, ['name', 'path']), request);
  }
  async createSkill(params: SkillCreateParams, request?: RequestOptions): Promise<SkillWriteResult> {
    const out = plain(params);
    out.name = newSkillName(out.name);
    return this.request('skills.create', out, request);
  }
  async updateSkill(params: SkillUpdateParams, request?: RequestOptions): Promise<SkillUpdateResult> {
    const out = plain(params, ['tier', 'name']);
    const patch = local(() => { object(out.patch); return { ...out.patch } as Record<string, unknown>; });
    if (patch.name !== undefined) patch.name = newSkillName(patch.name);
    return this.request('skills.update', { ...out, patch }, request);
  }
  async removeSkill(params: SkillRemoveParams, request?: RequestOptions): Promise<SkillRemoveResult> {
    return this.request('skills.remove', plain(params, ['tier', 'name']), request);
  }
  /** Install from base64 zip text (`data_b64`) or from an absolute file on the adapter host (`path`), not both. The text length is checked before anything is sent. */
  async installSkill(params: SkillInstallParams, request?: RequestOptions): Promise<SkillWriteResult> {
    const out = plain(params);
    const hasData = out.data_b64 !== undefined, hasPath = out.path !== undefined;
    if (hasData === hasPath) throw invalid('conflicting_params', 'Give exactly one of data_b64 and path');
    if (hasData) {
      textId(out.data_b64);
      if (out.data_b64.length > INLINE_ZIP_BASE64_CHARS) throw invalid('too_large', 'data_b64 exceeds 39146840 characters; install by path');
    } else textId(out.path);
    return this.request('skills.install', out, request);
  }
  /**
   * Install a skill from zip bytes. The size is checked against the adapter's inline limit (`limits.skills.inline_zip_bytes`,
   * 28 MiB by default) before encoding; a larger zip fails with `reason: 'too_large'` and nothing is sent. Use `installSkill({ path })` for those.
   */
  async installSkillZip(zip: Uint8Array, params: Omit<SkillInstallParams, 'data_b64' | 'path'>, request?: RequestOptions): Promise<SkillWriteResult> {
    if (!(zip instanceof Uint8Array) || zip.byteLength === 0) throw new SdkError('invalid_input', 'zip must be a nonempty Uint8Array');
    const cap = (await this.helloLimits()).skills?.inline_zip_bytes ?? INLINE_ZIP_BYTES;
    if (zip.byteLength > cap) throw invalid('too_large', `Zip is ${zip.byteLength} bytes; at most ${cap} can be sent inline`);
    const data_b64 = Buffer.from(zip.buffer, zip.byteOffset, zip.byteLength).toString('base64');
    return this.installSkill({ ...params, data_b64 }, request);
  }
  /** Rooms known to the registry, newest first, one page per call. */
  async listRooms(params?: RoomsListParams, request?: RequestOptions): Promise<RoomsListResult> {
    return this.request('room.list', plain(params), request);
  }
  /** The room template layer (`~/.koma/room`). A write applies to rooms started afterwards. */
  async getTeam(request?: RequestOptions): Promise<Team> {
    return this.request('room.team.get', { target: 'template' }, request);
  }
  async setTeamModel(params: Omit<TeamModelParams, 'target'>, request?: RequestOptions): Promise<TeamWriteResult> {
    return this.request('room.models.set', { ...plain(params, ['avatar']), target: 'template' }, request);
  }
  async upsertTeamAvatar(params: Omit<TeamAvatarUpsertParams, 'target'>, request?: RequestOptions): Promise<TeamWriteResult> {
    return this.request('room.avatars.upsert', { ...plain(params, ['id']), target: 'template' }, request);
  }
  async removeTeamAvatar(params: Omit<TeamAvatarRemoveParams, 'target'>, request?: RequestOptions): Promise<TeamAvatarRemoveResult> {
    return this.request('room.avatars.remove', { ...plain(params, ['id']), target: 'template' }, request);
  }
}

/** A local refusal that carries a reason from the registry, like the adapter's own `invalid_request` errors. */
function invalid(reason: string, message: string): SdkError { return Object.assign(new SdkError('invalid_input', message), { reason }); }

/**
 * Session-less calls on an UNBOUND adapter. The adapter holds no state, so it is started on first use and replaced
 * transparently once HOST_RECYCLE_AFTER requests were sent, or after it failed. A failed call is never retried.
 */
export class Host extends HostCalls {
  private current?: Promise<Transport>;
  private inflight = new Map<Transport, number>();
  private retired = new Set<Transport>();
  private closed = false;
  constructor(private readonly connect: () => Promise<Transport>, private readonly release: (t: Transport) => Promise<void>) { super(); }
  protected async helloLimits(): Promise<HelloLimits> { return (await this.hello()).limits; }
  private async transport(): Promise<Transport> {
    while (true) {
      if (this.closed) throw new SdkError('closed', 'Host is closed');
      const starting = this.current ??= this.connect();
      let t: Transport;
      try { t = await starting; }
      catch (e) { if (this.current === starting) this.current = undefined; throw e; }
      if (t.usable && t.requestsSent < Math.min(HOST_RECYCLE_AFTER, t.requestIdLimit)) return t;
      if (this.current === starting) { this.current = undefined; this.retire(t); }
    }
  }
  /** The old adapter finishes its in-flight requests before it is closed. */
  private retire(t: Transport): void {
    if (this.inflight.get(t)) this.retired.add(t);
    else void this.release(t);
  }
  protected async request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T> {
    const t = await this.transport();
    this.inflight.set(t, (this.inflight.get(t) ?? 0) + 1);
    try { return await t.request<T>(method, params, options); }
    finally {
      const left = (this.inflight.get(t) ?? 1) - 1;
      if (left) this.inflight.set(t, left);
      else { this.inflight.delete(t); if (this.retired.delete(t)) void this.release(t); }
    }
  }
  /** hello of the current adapter. */
  async hello(): Promise<Hello> { return (await this.transport()).hello!; }
  /** Whether the adapter implements a method: hello `methods`, or `capabilities` for an older adapter. */
  async supports(method: string): Promise<boolean> { return (await this.transport()).supports(method); }
  /** Write providers and model roles. No daemon is started. */
  async configure(options: ConfigureOptions, request?: RequestOptions): Promise<ConfigureResult> {
    return this.request('configure', configureParams(options), request);
  }
  /** Read config.json as a redacted view. Secret values are never returned. */
  async getConfig(params?: ConfigGetParams, request?: RequestOptions): Promise<ConfigView> {
    return this.request('config.get', plain(params), request);
  }
  async deleteProvider(params: ProviderDeleteParams, request?: RequestOptions): Promise<ProviderDeleteResult> {
    return this.request('config.providers.delete', plain(params, ['provider_uuid']), request);
  }
  async deleteModel(params: ModelDeleteParams, request?: RequestOptions): Promise<ModelDeleteResult> {
    return this.request('config.models.delete', plain(params, ['model_uuid']), request);
  }
  /** Clear a role globally (scope "global" unless given). */
  async unassignRole(params: RoleUnassignParams, request?: RequestOptions): Promise<RoleUnassignResult> {
    return this.request('config.roles.unassign', unassignParams(params), request);
  }
  async close(): Promise<void> {
    this.closed = true;
    const current = this.current; this.current = undefined;
    const retired = [...this.retired]; this.retired.clear();
    await Promise.all([
      ...retired.map(t => this.release(t)),
      ...(current ? [current.then(t => this.release(t), () => undefined)] : []),
    ]);
  }
  async [Symbol.asyncDispose](): Promise<void> { await this.close(); }
}

export class Session extends HostCalls {
  private latestInfo: SessionInfo | null;
  constructor(private readonly client: Client, private readonly transport: Transport, private state: State, info: SessionInfo | null = null, private readonly createWarnings: Warning[] = []) {
    super();
    this.latestInfo = info;
  }
  /** Warnings of the `create` call, for example `rename_failed` when `name` could not be applied. */
  get warnings(): readonly Warning[] { return this.createWarnings; }
  protected async helloLimits(): Promise<HelloLimits> { return this.transport.hello!.limits; }
  get sessionId(): string { return this.state.session_id; }
  get stderrTail(): string { return this.transport.diagnostics; }
  /** Latest per-session facts, or null when the adapter sends none (an older adapter). */
  get sessionInfo(): SessionInfo | null { return this.latestInfo; }
  /** Requests left before this adapter's request-id limit. Reconnect with resume from a new client before it reaches 0. */
  get requestsRemaining(): number { return this.transport.requestsRemaining; }
  /** Whether the adapter implements a method: hello `methods`, or `capabilities` for an older adapter. */
  supports(method: string): boolean { return this.transport.supports(method); }
  protected async request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T> {
    try { return await this.transport.request<T>(method, params, options); }
    catch (e) { if (e instanceof DeliveryUnknownError) e.sessionId = this.sessionId; throw e; }
  }
  /** session_info is ordered by observed_at_ms, independent of the state revision. */
  private observe<T>(result: T): T {
    const info = (result as { session_info?: SessionInfo }).session_info;
    if (info && (!this.latestInfo || info.observed_at_ms >= this.latestInfo.observed_at_ms)) this.latestInfo = info;
    return result;
  }
  async getState(options?: RequestOptions): Promise<State> {
    const result = this.observe(await this.request<{ state: State; session_info?: SessionInfo }>('state', {}, options));
    if (result.state.revision > this.state.revision) this.state = result.state;
    return this.state;
  }
  /** Read fresh session_info. The adapter answers from what it last observed; no daemon round trip. */
  async info(options?: RequestOptions): Promise<SessionInfo> {
    this.observe(await this.request<{ session_info: SessionInfo }>('sessions.info', {}, options));
    return this.latestInfo!;
  }
  /** Read config. The default view on a session is "session" (with the session's own role routes). */
  async getConfig(params?: ConfigGetParams, request?: RequestOptions): Promise<ConfigView> {
    return this.request('config.get', plain(params), request);
  }
  async deleteProvider(params: ProviderDeleteParams, request?: RequestOptions): Promise<ProviderDeleteResult> {
    return this.observe(await this.request<ProviderDeleteResult>('config.providers.delete', plain(params, ['provider_uuid']), request));
  }
  async deleteModel(params: ModelDeleteParams, request?: RequestOptions): Promise<ModelDeleteResult> {
    return this.observe(await this.request<ModelDeleteResult>('config.models.delete', plain(params, ['model_uuid']), request));
  }
  /** Clear a role globally, as Host.unassignRole does (scope "global" unless given). To drop this session's own Main pin, use clearMainPin. */
  async unassignRole(params: RoleUnassignParams, request?: RequestOptions): Promise<RoleUnassignResult> {
    return this.observe(await this.request<RoleUnassignResult>('config.roles.unassign', unassignParams(params), request));
  }
  /** Clear this session's Main pin, so Main follows the global assignment. Resets effort. */
  async clearMainPin(request?: RequestOptions): Promise<RoleUnassignResult> {
    return this.observe(await this.request<RoleUnassignResult>('config.roles.unassign', { role: 'main', scope: 'session' }, request));
  }
  async getEffort(params?: EffortGetParams, request?: RequestOptions): Promise<EffortOptions> {
    return this.request('config.effort.get', plain(params), request);
  }
  async setEffort(params: EffortSetParams, request?: RequestOptions): Promise<EffortResult> {
    return this.observe(await this.request<EffortResult>('config.effort.set', plain(params, ['effort']), request));
  }
  async getLimits(request?: RequestOptions): Promise<Limits> { return this.request('config.limits.get', {}, request); }
  async setLimits(params: LimitsSetParams, request?: RequestOptions): Promise<LimitsResult> {
    return this.observe(await this.request<LimitsResult>('config.limits.set', plain(params), request));
  }
  async *events(options?: Pick<RequestOptions, 'signal'>): AsyncGenerator<Event> {
    for await (const event of this.transport.events(options, () => this.getState(options))) {
      if (isRoomEvent(event)) continue;
      if (event.gap) {
        yield { type: 'event', event: 'state', seq: event.seq, state: await this.getState(), resynced: true }; continue;
      }
      if (event.event === 'state') {
        if (event.state.revision < this.state.revision || (event.state.revision === this.state.revision && !event.resynced)) continue;
        this.state = event.state;
      } else if (event.event === 'output_append') {
        if (event.revision <= this.state.revision) continue;
        if (event.revision !== this.state.revision + 1 || event.offset !== Buffer.byteLength(this.state.streaming ?? '')) {
          const state = await this.getState();
          yield { type: 'event', event: 'state', seq: event.seq, state, resynced: true }; continue;
        }
        this.state = { ...this.state, revision: event.revision, streaming: (this.state.streaming ?? '') + event.text };
      } else if (event.revision !== this.state.revision) continue;
      yield event;
    }
  }
  /** Update providers or model roles on this live session. The next turn uses them. */
  configure(options: ConfigureOptions, request?: RequestOptions): Promise<ConfigureResult> {
    return this.request('configure', configureParams(options), request);
  }
  submit(input: Input, options?: RequestOptions): Promise<SubmitResult> { return this.request('submit', validateInput(input), options); }
  enqueue(input: Input, options?: RequestOptions): Promise<EnqueueResult> { return this.request('enqueue', validateInput(input), options); }
  interrupt(turnId: string, options?: RequestOptions): Promise<InterruptResult> { textId(turnId); return this.request('interrupt', { turn_id: turnId }, options); }
  interruptAndSubmit(turnId: string, input: Input, options?: RequestOptions): Promise<ReplacementResult> {
    textId(turnId); return this.request('interrupt_and_submit', { ...validateInput(input), turn_id: turnId }, options);
  }
  approve(callId: string, approve: boolean, options?: RequestOptions): Promise<{ approved: boolean }> {
    textId(callId);
    if (!this.state.external_approvals || typeof approve !== 'boolean') throw new SdkError('invalid_input', 'External tool approvals require opt-in and a boolean decision');
    return this.request('approve', { call_id: callId, approve }, options);
  }
  async waitTurn(turnId: string, options: RequestOptions = {}): Promise<Turn> {
    textId(turnId); const deadline = Date.now() + (options.timeoutMs ?? 60_000);
    while (Date.now() < deadline) {
      const state = await this.getState({ ...options, timeoutMs: Math.max(1, deadline - Date.now()) });
      if (state.turn?.id === turnId && state.turn.state !== 'running') return state.turn;
      if (state.turn && state.turn.id !== turnId) throw new TurnReplacedError(turnId, state.turn.id);
      await new Promise(resolve => setTimeout(resolve, Math.min(100, Math.max(0, deadline - Date.now()))));
    }
    throw new SdkError('observation_unavailable', 'Terminal state not observed before timeout; work may continue');
  }
  async detach(options?: RequestOptions): Promise<void> {
    try { await this.request('detach', {}, options); } finally { await this.close(); }
  }
  async close(): Promise<void> { await this.client.release(this.transport); }
  async [Symbol.asyncDispose](): Promise<void> { await this.close(); }
}

/**
 * A bound room: one adapter holds the live view of one room. The room has a single live subscriber, so attaching from
 * elsewhere (another `Room`, the terminal) ends this view with `room.closed` `taken_over`. The library never re-attaches
 * by itself; call `reattach()` to take the view back. Reads and file-only team writes keep working after the view ended.
 */
export class Room extends HostCalls {
  private attached: AttachResult;
  private lastLine: number;
  private binding?: Promise<unknown>;
  constructor(private readonly client: Client, private readonly transport: Transport, attached: AttachResult) {
    super();
    this.attached = attached;
    this.lastLine = attached.history.last_line_seq ?? 0;
  }
  /** The room's session id. */
  get sessionId(): string { return this.attached.room.session_id; }
  /** Id of the current line numbering. `line_seq` values only compare within one epoch. */
  get epoch(): string { return this.attached.epoch; }
  /** The result of the latest attach: room, daemon facts, tail, status and roster. */
  get attachResult(): AttachResult { return this.attached; }
  get stderrTail(): string { return this.transport.diagnostics; }
  /** Requests left before this adapter's request-id limit. To go on, `roomAttach` from a new client; this one then sees `taken_over`. */
  get requestsRemaining(): number { return this.transport.requestsRemaining; }
  supports(method: string): boolean { return this.transport.supports(method); }
  protected async helloLimits(): Promise<HelloLimits> { return this.transport.hello!.limits; }
  protected request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T> {
    return this.transport.request<T>(method, params, options);
  }
  /** Send text to the room. Not retried: a lost reply throws `DeliveryUnknownError`. */
  async send(params: RoomSendParams, request?: RequestOptions): Promise<RoomSendResult> {
    local(() => { object(params); if (params.to !== undefined) string(params.to, true); });
    const out: Record<string, unknown> = validateInput({ text: params.text, attachments: params.attachments });
    if (params.to !== undefined) out.to = params.to;
    if (params.shell !== undefined) out.shell = params.shell;
    return this.request('room.send', out, request);
  }
  /** Stops every chat lane; work lanes continue. */
  async interrupt(request?: RequestOptions): Promise<{ sent: true }> { return this.request('room.interrupt', {}, request); }
  async control(params: RoomControlParams, request?: RequestOptions): Promise<{ sent: true }> {
    return this.request('room.control', plain(params, ['avatar']), request);
  }
  async status(request?: RequestOptions): Promise<RoomStatus> { return this.request('room.status', {}, request); }
  /** Lines of the adapter's ring. `epoch` defaults to the current one. */
  async history(params: RoomHistoryParams = {}, request?: RequestOptions): Promise<RoomHistoryResult> {
    return this.request('room.history', { ...plain(params), epoch: params.epoch ?? this.epoch }, request);
  }
  async shelf(params?: ShelfListParams, request?: RequestOptions): Promise<ShelfListResult> { return this.request('room.shelf.list', plain(params), request); }
  async item(key: string, request?: RequestOptions): Promise<ShelfItem> { textId(key); return this.request('room.item', { key }, request); }
  async trajectory(params?: GoalParams, request?: RequestOptions): Promise<TrajectoryResult> { return this.request('room.trajectory', plain(params), request); }
  async reports(params?: ReportsParams, request?: RequestOptions): Promise<ReportsResult> { return this.request('room.reports', plain(params), request); }
  async spend(params?: GoalParams, request?: RequestOptions): Promise<SpendResult> { return this.request('room.spend', plain(params), request); }
  /** The team of this room (`target` "room", the default) or of the template layer. */
  async team(params?: TeamGetParams, request?: RequestOptions): Promise<Team> {
    return this.request('room.team.get', { target: 'room', ...plain(params) }, request);
  }
  async setModel(params: Omit<TeamModelParams, 'target'> & { target?: TeamTarget }, request?: RequestOptions): Promise<TeamWriteResult> {
    return this.request('room.models.set', { target: 'room', ...plain(params, ['avatar']) }, request);
  }
  async upsertAvatar(params: Omit<TeamAvatarUpsertParams, 'target'> & { target?: TeamTarget }, request?: RequestOptions): Promise<TeamWriteResult> {
    return this.request('room.avatars.upsert', { target: 'room', ...plain(params, ['id']) }, request);
  }
  async removeAvatar(params: Omit<TeamAvatarRemoveParams, 'target'> & { target?: TeamTarget }, request?: RequestOptions): Promise<TeamAvatarRemoveResult> {
    return this.request('room.avatars.remove', { target: 'room', ...plain(params, ['id']) }, request);
  }
  /** Take the live view back after `room.closed`. Starts a new epoch and replaces what `epoch` and `attachResult` report. */
  async reattach(params: { tail?: number; wait_ms?: number } = {}, request?: RequestOptions): Promise<AttachResult> {
    const timeoutMs = Math.max(30_000, (params.wait_ms ?? 2_000) + 5_000);
    const call = this.request<AttachResult>('room.attach', { ...plain(params), session_id: this.sessionId }, { timeoutMs, ...request });
    this.binding = call;
    try {
      this.attached = await call;
      this.lastLine = this.attached.history.last_line_seq ?? 0;
      return this.attached;
    } finally { this.binding = undefined; }
  }
  /**
   * Start the room again (`room.open` on its parent) after its daemon exited, and take the view. Starts a new epoch like
   * `reattach`. Fails locally when the room's parent session is not known.
   */
  async reopen(params: { tail?: number; wait_ms?: number } = {}, request?: RequestOptions): Promise<AttachResult> {
    const parent = this.attached.room.parent_session_id;
    if (parent === null || parent === undefined) throw new SdkError('invalid_input', 'The room has no known parent session; use Client.roomOpen');
    const timeoutMs = Math.max(30_000, (params.wait_ms ?? 12_000) + 10_000);
    const call = this.request<AttachResult>('room.open', { ...plain(params), parent_session_id: parent }, { timeoutMs, ...request });
    this.binding = call;
    try {
      this.attached = await call;
      this.lastLine = this.attached.history.last_line_seq ?? 0;
      return this.attached;
    } finally { this.binding = undefined; }
  }
  /**
   * Events of the current epoch, in order. Lines the view missed (`room.gap`, a client-side overflow, a hole in the envelope
   * sequence) are read back with `room.history` and yielded as `room.line` events with `recovered: true`, followed by the fresh
   * status. The iterator ends after `room.closed`; it does not re-attach. One iterator at a time.
   */
  async *events(options?: Pick<RequestOptions, 'signal'>): AsyncGenerator<RoomEvent> {
    const recover = async (): Promise<Incoming[]> => this.catchUp(0);
    while (true) {
      try {
        for await (const event of this.transport.events(options, recover)) {
          if (!isRoomEvent(event)) continue;
          if (event.epoch !== this.epoch) {
            // A reattach in flight decides which epoch is current.
            await this.binding?.catch(() => undefined);
            if (event.epoch !== this.epoch) continue;
          }
          const recovered = 'recovered' in event && event.recovered === true;
          if (event.gap && !recovered) yield* await this.catchUp(event.seq);
          if (event.event === 'room.line' && !recovered) {
            if (event.line.line_seq <= this.lastLine) continue;
            this.lastLine = event.line.line_seq;
          }
          yield event;
          if (event.event === 'room.gap') yield* await this.catchUp(event.seq);
          if (event.event === 'room.closed') return;
        }
        return;
      } catch (e) {
        if (!(e instanceof SdkError) || e.code !== 'event_overflow') throw e;
      }
    }
  }
  /** Lines after the last one seen, then the current status. */
  private async catchUp(seq: number): Promise<RoomEvent[]> {
    const epoch = this.epoch;
    const out: RoomEvent[] = [];
    while (true) {
      const page = await this.history({ epoch, after_line_seq: this.lastLine, limit: 200 });
      for (const line of page.lines) {
        if (line.line_seq <= this.lastLine) continue;
        this.lastLine = line.line_seq;
        out.push({ type: 'event', event: 'room.line', seq, epoch, line, recovered: true });
      }
      if (!page.has_more_after || !page.lines.length) break;
    }
    out.push({ type: 'event', event: 'room.status', seq, epoch, status: await this.status(), recovered: true });
    return out;
  }
  /**
   * Leave the room and release the adapter: sends `room.close` (`stop: true` also stops the room's daemon), then closes the
   * adapter. Returns undefined when the adapter had already failed.
   */
  async close(options: { stop?: boolean } & RequestOptions = {}): Promise<RoomCloseResult | undefined> {
    const { stop, ...request } = options;
    try { return this.transport.usable ? await this.request<RoomCloseResult>('room.close', stop === undefined ? {} : { stop }, request) : undefined; }
    finally { await this.client.release(this.transport); }
  }
  async [Symbol.asyncDispose](): Promise<void> { await this.close().catch(() => undefined); }
}
