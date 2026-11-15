import { isAbsolute } from 'node:path';
import { Transport } from './transport.js';
import { configureParams, local, object, textId, validateInput, validatePolicy } from './validate.js';
import { DeliveryUnknownError, SdkError, TurnReplacedError, type ClientOptions, type ConfigureOptions, type ConfigureResult, type CreateOptions, type ResumeOptions, type RequestOptions, type State, type Hello, type Input, type SubmitResult, type EnqueueResult, type InterruptResult, type ReplacementResult, type Event, type Turn, type Method, type SessionInfo, type ConfigView, type ConfigGetParams, type ProviderDeleteParams, type ProviderDeleteResult, type ModelDeleteParams, type ModelDeleteResult, type RoleUnassignParams, type RoleUnassignResult, type EffortGetParams, type EffortOptions, type EffortSetParams, type EffortResult, type Limits, type LimitsSetParams, type LimitsResult } from './types.js';
export * from './types.js';

/** A stateless adapter is replaced once this many requests were sent, well before the per-process cap. */
export const HOST_RECYCLE_AFTER = 60_000;

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
    return this.bind('create', { ...options, policy }, request);
  }
  async resume(options: ResumeOptions, request?: RequestOptions): Promise<Session> {
    textId(options.session_id);
    if (options.policy) {
      local(() => validatePolicy(options.policy));
      if (options.policy.approval === 'default' && !options.external_approvals) throw new SdkError('invalid_input', 'approval default requires external_approvals');
    }
    return this.bind('resume', { ...options }, request);
  }
  private async bind(method: 'create' | 'resume', params: Record<string, unknown>, options?: RequestOptions): Promise<Session> {
    const t = await this.connect();
    try {
      const result = await t.request<{ state: State; session_info?: SessionInfo }>(method, params, options);
      return new Session(this, t, result.state, result.session_info ?? null);
    } catch (e) { await this.release(t); throw e; }
  }
  async close(): Promise<void> {
    this.closed = true;
    await Promise.all([...this.connections].map(t => this.release(t)));
  }
  async [Symbol.asyncDispose](): Promise<void> { await this.close(); }
}

/**
 * Session-less calls on an UNBOUND adapter. The adapter holds no state, so it is started on first use and replaced
 * transparently once HOST_RECYCLE_AFTER requests were sent, or after it failed. A failed call is never retried.
 */
export class Host {
  private current?: Promise<Transport>;
  private inflight = new Map<Transport, number>();
  private retired = new Set<Transport>();
  private closed = false;
  constructor(private readonly connect: () => Promise<Transport>, private readonly release: (t: Transport) => Promise<void>) {}
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
  private async request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T> {
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

export class Session {
  private latestInfo: SessionInfo | null;
  constructor(private readonly client: Client, private readonly transport: Transport, private state: State, info: SessionInfo | null = null) {
    this.latestInfo = info;
  }
  get sessionId(): string { return this.state.session_id; }
  get stderrTail(): string { return this.transport.diagnostics; }
  /** Latest per-session facts, or null when the adapter sends none (an older adapter). */
  get sessionInfo(): SessionInfo | null { return this.latestInfo; }
  /** Requests left before this adapter's request-id limit. Reconnect with resume from a new client before it reaches 0. */
  get requestsRemaining(): number { return this.transport.requestsRemaining; }
  /** Whether the adapter implements a method: hello `methods`, or `capabilities` for an older adapter. */
  supports(method: string): boolean { return this.transport.supports(method); }
  private async request<T>(method: Method, params: Record<string, unknown>, options?: RequestOptions): Promise<T> {
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
