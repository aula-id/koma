import { isAbsolute } from 'node:path';
import { ROOM_METHODS, SKILL_METHODS, SdkError, type ConfigureOptions, type ErrorDetails, type Hello, type Method, type ModelRoleName, type Policy, type Input, type V1Method } from './types.js';
export function object(value: unknown): asserts value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new SdkError('protocol', 'Expected object');
}
export function string(value: unknown, nonempty = false): asserts value is string {
  if (typeof value !== 'string' || (nonempty && !value.length)) throw new SdkError('protocol', 'Expected string');
}
function nullableString(x: unknown): void { if (x !== null) string(x); }
function bool(x: unknown): void { if (typeof x !== 'boolean') throw new SdkError('protocol', 'Expected boolean'); }
function member(x: unknown, allowed: readonly unknown[]): void { if (!allowed.includes(x)) throw new SdkError('protocol', `Unexpected enum: ${String(x)}`); }
export function validatePolicy(x: unknown): asserts x is Policy { object(x); member(x.plan, ['allow', 'forbid']); member(x.approval, ['default', 'reject']); }
export function validateState(x: unknown): void {
  object(x);
  if (!Number.isSafeInteger(x.revision) || (x.revision as number) < 1) throw new SdkError('protocol', 'Invalid state revision');
  bool(x.external_approvals);
  if (x.pending_approval !== null) { object(x.pending_approval); string(x.pending_approval.call_id, true); nullableString(x.pending_approval.reason); }
  if (x.gate !== null) { object(x.gate); member(x.gate.kind, ['tool','plan','mission','unknown']); nullableString(x.gate.reason); bool(x.gate.supported); }
  string(x.session_id, true); string(x.cwd, true); string(x.model_id); validatePolicy(x.policy);
  if (x.turn !== null) { object(x.turn); string(x.turn.id, true); member(x.turn.state, ['running', 'finished', 'interrupted', 'failed']); nullableString(x.turn.error); }
  bool(x.working); bool(x.awaiting_approval); nullableString(x.approval_reason); nullableString(x.streaming);
  if (!Number.isInteger(x.pending_count) || (x.pending_count as number) < 0 || (x.pending_count as number) > 5) throw new SdkError('protocol', 'Invalid pending_count');
  if (!Array.isArray(x.messages)) throw new SdkError('protocol', 'Expected messages array');
  for (const m of x.messages) { object(m); member(m.role, ['system', 'user', 'assistant', 'tool']); string(m.content); }
}
function names(x: unknown, what: string): x is string[] {
  if (!Array.isArray(x) || !x.every(c => typeof c === 'string') || new Set(x).size !== x.length) throw new SdkError('protocol', `Invalid ${what}`);
  return true;
}
/** Validates the hello frame and returns it with `methods`, `features`, `events` and `limits` filled in for an older adapter. */
export function validateHello(x: Record<string, unknown>, required: readonly V1Method[]): Hello {
  if (x.type !== 'hello' || x.protocol !== 1) throw new SdkError('incompatible_protocol', 'First frame must be a protocol 1 hello');
  object(x.binary); string(x.binary.path, true); string(x.binary.version, true);
  if (!Array.isArray(x.diagnostics) || !x.diagnostics.every(d => typeof d === 'string')) throw new SdkError('protocol', 'Invalid diagnostics');
  names(x.capabilities, 'capabilities');
  const missing = required.filter(m => !(x.capabilities as unknown[]).includes(m));
  if (missing.length) throw new SdkError('incompatible_capabilities', `Missing required capabilities: ${missing.join(', ')}`);
  for (const key of ['methods', 'features', 'events'] as const) if (x[key] !== undefined) names(x[key], key);
  if (x.limits !== undefined) object(x.limits);
  return { ...x, methods: x.methods ?? [], features: x.features ?? [], events: x.events ?? [], limits: x.limits ?? {} } as unknown as Hello;
}
/** Methods an adapter implements: `methods`, or `capabilities` for an older adapter without it. */
export function implementedMethods(hello: Hello): readonly string[] {
  return hello.methods.length ? hello.methods : hello.capabilities;
}
/** Reads the optional error keys; a present key of the wrong type is a protocol error. */
export function errorDetails(x: Record<string, unknown>): ErrorDetails {
  const out: ErrorDetails = {};
  if (x.reason !== undefined) { string(x.reason, true); out.reason = x.reason; }
  if (x.binding !== undefined) { string(x.binding, true); out.binding = x.binding; }
  if (x.allowed !== undefined && names(x.allowed, 'allowed')) out.allowed = [...x.allowed];
  if (x.written !== undefined) { bool(x.written); out.written = x.written as boolean; }
  if (x.detail !== undefined) { object(x.detail); out.detail = x.detail; }
  return out;
}
export function validateEvent(x: Record<string, unknown>): void {
  if (typeof x.event === 'string' && x.event.startsWith('room.')) { validateRoomEvent(x); return; }
  if (x.event === 'state') { validateState(x.state); bool(x.resynced); return; }
  if (!Number.isSafeInteger(x.revision) || (x.revision as number) < 1) throw new SdkError('protocol', 'Invalid event revision');
  if (x.event === 'output_append') {
    if (!Number.isSafeInteger(x.offset) || (x.offset as number) < 0) throw new SdkError('protocol', 'Invalid append offset');
    string(x.text, true);
  } else if (x.turn !== null) { object(x.turn); string(x.turn.id, true); member(x.turn.state, ['running','finished','interrupted','failed']); nullableString(x.turn.error); }
}
function validateRoomEvent(x: Record<string, unknown>): void {
  string(x.epoch, true);
  switch (x.event) {
    case 'room.line': validateRoomLine(x.line); return;
    case 'room.delta': string(x.author, true); string(x.lane, true); string(x.kind, true); string(x.text); return;
    case 'room.status': validateRoomStatus(x.status); return;
    case 'room.notice': string(x.code, true); string(x.text); return;
    case 'room.closed': string(x.session_id, true); string(x.reason, true); return;
    case 'room.gap': int(x.after_line_seq); int(x.dropped_lines); int(x.dropped_deltas); return;
  }
}
const ROLES: readonly ModelRoleName[] = ['main', 'awareness', 'safeguard', 'compactor', 'planner'];
export function configureParams(options: ConfigureOptions): Record<string, unknown> {
  return local(() => {
    object(options);
    const params: Record<string, unknown> = {};
    if (options.providers !== undefined) {
      if (!Array.isArray(options.providers) || options.providers.length > 32) throw new Error('providers must be a list of at most 32');
      const seen = new Set<string>();
      params.providers = options.providers.map(provider => {
        object(provider);
        string(provider.name, true);
        string(provider.endpoint, true);
        const name = provider.name.trim();
        if (!name) throw new Error('provider name is empty');
        if (seen.has(name)) throw new Error(`duplicate provider name '${name}'`);
        seen.add(name);
        if (provider.api_type !== 'openai' && provider.api_type !== 'anthropic') throw new Error("api_type must be 'openai' or 'anthropic'");
        const entry: Record<string, unknown> = { name, endpoint: provider.endpoint.trim(), api_type: provider.api_type };
        if (provider.api_key !== undefined && provider.api_key !== null) {
          if (typeof provider.api_key !== 'string' || !provider.api_key.trim()) throw new Error('api_key must be non-empty when provided');
          entry.api_key = provider.api_key.trim();
        }
        return entry;
      });
    }
    if (options.models !== undefined) {
      object(options.models);
      const models: Record<string, unknown> = {};
      for (const [role, spec] of Object.entries(options.models)) {
        if (!ROLES.includes(role as ModelRoleName)) throw new Error(`unknown model role '${role}'`);
        if (spec === undefined) continue;
        object(spec);
        string(spec.provider, true);
        string(spec.model_id, true);
        const provider = spec.provider.trim();
        const modelId = spec.model_id.trim();
        if (!provider || !modelId) throw new Error('model provider and model_id are required');
        const assignment: Record<string, unknown> = { provider, model_id: modelId };
        if (spec.route !== undefined) { string(spec.route); assignment.route = spec.route; }
        models[role] = assignment;
      }
      params.models = models;
    }
    const listed = params.providers as unknown[] | undefined;
    const assigned = params.models as Record<string, unknown> | undefined;
    if ((!listed || listed.length === 0) && (!assigned || Object.keys(assigned).length === 0)) throw new Error('providers or models required');
    return params;
  });
}
function validateConfigure(result: Record<string, unknown>): void {
  if (!Array.isArray(result.providers) || !result.models || typeof result.models !== 'object' || Array.isArray(result.models)) throw new SdkError('protocol', 'Invalid configure result');
  for (const provider of result.providers) {
    object(provider); string(provider.name, true); string(provider.uuid, true);
    if ('api_key' in provider || 'key' in provider) throw new SdkError('protocol', 'Configure result must not include a key');
  }
  for (const [role, spec] of Object.entries(result.models)) {
    if (!ROLES.includes(role as ModelRoleName)) throw new SdkError('protocol', 'Invalid configure role');
    object(spec); string(spec.uuid, true); string(spec.model_id, true); string(spec.provider, true);
    if ('api_key' in spec || 'key' in spec) throw new SdkError('protocol', 'Configure result must not include a key');
  }
}
function int(x: unknown): void { if (!Number.isSafeInteger(x) || (x as number) < 0) throw new SdkError('protocol', 'Expected non-negative integer'); }
function nullableInt(x: unknown): void { if (x !== null) int(x); }
function strings(x: unknown, nonempty = false): void {
  if (!Array.isArray(x)) throw new SdkError('protocol', 'Expected array');
  for (const item of x) string(item, nonempty);
}
function list(x: unknown, item: (v: unknown) => void): void {
  if (!Array.isArray(x)) throw new SdkError('protocol', 'Expected array');
  x.forEach(item);
}
// Enum-like strings added after v1 are open: any string passes. PublicRole stays closed.
function role(x: unknown): void { member(x, ROLES); }
function noSecret(x: Record<string, unknown>, keys: readonly string[]): void {
  if (keys.some(k => k in x)) throw new SdkError('protocol', 'Result must not include a secret value');
}
function validateRoute(x: unknown): void {
  object(x); role(x.role); string(x.origin, true);
  for (const k of ['model_uuid', 'source_uuid', 'configured_model', 'provider_uuid', 'provider_name', 'effective_model', 'reason']) nullableString(x[k]);
}
export function validateConfigView(x: unknown): void {
  object(x); string(x.revision, true); string(x.file, true); string(x.view, true);
  list(x.providers, p => {
    object(p); noSecret(p, ['api_key', 'key']);
    string(p.provider_uuid, true); string(p.name); string(p.api_type, true); string(p.endpoint);
    bool(p.endpoint_redacted); bool(p.managed); bool(p.has_key); string(p.key_state, true); int(p.model_count);
  });
  list(x.oauth, o => {
    object(o); noSecret(o, ['token', 'access_token', 'refresh_token', 'id_token']);
    string(o.oauth_uuid, true); string(o.name); string(o.provider); bool(o.has_token); int(o.expires_at);
  });
  list(x.models, m => {
    object(m); string(m.model_uuid, true); string(m.name); string(m.model_id); string(m.provider_uuid);
    string(m.provider_kind, true); nullableString(m.provider_name); nullableString(m.route);
    list(m.roles, role); bool(m.other_roles); nullableString(m.source_uuid);
  });
  bool(x.providers_truncated); bool(x.oauth_truncated); bool(x.models_truncated);
  list(x.roles, validateRoute); nullableString(x.main_fallback);
}
function validateLimits(x: unknown): void {
  object(x); int(x.max_output_tokens); int(x.context_window_limit); string(x.context_model_alias); nullableInt(x.effective_window);
}
export function validateSessionInfo(x: unknown): void {
  object(x); string(x.name); string(x.effort, true); list(x.routes, validateRoute); validateLimits(x.limits);
  if (x.usage !== null) {
    const u = x.usage; object(u);
    int(u.last_prompt_tokens); int(u.last_cached_tokens); int(u.output_tokens_total);
    if (typeof u.cost_total !== 'number' || !Number.isFinite(u.cost_total) || u.cost_total < 0) throw new SdkError('protocol', 'Invalid cost_total');
    if (u.context !== null) {
      const c = u.context; object(c);
      int(c.prompt_tokens); int(c.effective_window); bool(c.estimated); nullableInt(c.cached_tokens); bool(c.drss_active);
    }
  }
  object(x.daemon); string(x.daemon.version); int(x.daemon.ipc_protocol); strings(x.daemon.capabilities, true);
  int(x.observed_at_ms);
}
function optionalInfo(x: Record<string, unknown>): void { if (x.session_info !== undefined) validateSessionInfo(x.session_info); }
function validatePropagation(x: unknown): void {
  object(x);
  for (const value of Object.values(x)) string(value, true);
}
function validateMutation(x: Record<string, unknown>, view: unknown): void {
  validateConfigView(x.config); validatePropagation(x.propagation); optionalInfo(x);
  if ((x.config as Record<string, unknown>).view !== view) throw new SdkError('protocol', 'Config view does not match the call');
}
function validateDelete(x: Record<string, unknown>, params: Record<string, unknown>, key: 'provider_uuid' | 'model_uuid'): void {
  bool(x.dry_run);
  if (x.dry_run !== (params.dry_run === true)) throw new SdkError('protocol', 'dry_run does not match the call');
  object(x.removed);
  if (x.removed[key] !== params[key]) throw new SdkError('protocol', 'Deleted entry does not match the call');
  if (key === 'provider_uuid') strings(x.removed.models_removed, true);
  else list(x.removed.roles_released, role);
  bool(x.removed.main_reset);
  validateMutation(x, 'global');
}
function nullableBool(x: unknown): void { if (x !== null) bool(x); }
function num(x: unknown): void { if (typeof x !== 'number' || !Number.isFinite(x) || x < 0) throw new SdkError('protocol', 'Expected non-negative number'); }
function fields(x: Record<string, unknown>, nullable: readonly string[]): void { for (const k of nullable) nullableString(x[k]); }
function cursorPage(x: Record<string, unknown>, key: string, item: (v: unknown) => void, max: number): void {
  list(x[key], item); if ((x[key] as unknown[]).length > max) throw new SdkError('protocol', `Too many ${key}`);
  nullableString(x.next_cursor);
}
function validateSessionRow(x: unknown): void {
  object(x); string(x.session_id, true); fields(x, ['name', 'workdir', 'parent_session_id', 'room_session_id']); string(x.kind, true);
  nullableInt(x.created_at); nullableInt(x.updated_at); bool(x.live); nullableBool(x.working); bool(x.locked); bool(x.bound); bool(x.room_live);
}
function validateTranscript(x: Record<string, unknown>, params: Record<string, unknown>): void {
  string(x.session_id, true); bool(x.live);
  if (x.session_id !== params.session_id) throw new SdkError('protocol', 'Transcript does not match the call');
  list(x.messages, m => {
    object(m);
    if (!Number.isSafeInteger(m.id) || (m.id as number) < 1) throw new SdkError('protocol', 'Invalid message id');
    member(m.role, ['system', 'user', 'assistant', 'tool']); string(m.text); int(m.created_at);
    if (m.truncated !== undefined && m.truncated !== true) throw new SdkError('protocol', 'truncated must be true when present');
  });
  if ((x.messages as unknown[]).length > 500) throw new SdkError('protocol', 'Too many messages');
  if (x.next_since !== null && (!Number.isSafeInteger(x.next_since) || (x.next_since as number) < 1)) throw new SdkError('protocol', 'Invalid next_since');
}
function validateTotals(x: unknown): void {
  object(x); int(x.calls); int(x.tokens_in); int(x.tokens_cached); int(x.tokens_out); num(x.cost);
}
function validateUsage(x: Record<string, unknown>): void {
  if (!Number.isSafeInteger(x.from_at) || !Number.isSafeInteger(x.to_at)) throw new SdkError('protocol', 'Invalid usage window');
  string(x.ledger, true); validateTotals(x.totals);
  if (x.groups !== undefined) {
    list(x.groups, g => {
      object(g); validateTotals(g); string(g.key); nullableString(g.label);
      if (g.role_kind !== undefined) string(g.role_kind, true);
      if (g.agent !== undefined) nullableString(g.agent);
      if (g.kind !== undefined) nullableString(g.kind);
      if (g.deleted !== undefined) bool(g.deleted);
    });
    bool(x.groups_truncated);
  }
  if (x.buckets !== undefined) list(x.buckets, b => { object(b); validateTotals(b); if (!Number.isSafeInteger(b.start_at)) throw new SdkError('protocol', 'Invalid bucket start'); });
}
function validateSkillRow(x: unknown): void {
  object(x); string(x.name, true); string(x.scope, true); string(x.tier, true); string(x.description);
  strings(x.triggers); bool(x.triggers_truncated); strings(x.tags); bool(x.tags_truncated);
  bool(x.pinned); bool(x.editable); string(x.generation, true); string(x.path); bool(x.effective); nullableString(x.shadowed_by);
}
function validateSkillDetail(x: unknown): void {
  validateSkillRow(x); object(x);
  string(x.instruction); strings(x.declared_tools); bool(x.declared_tools_truncated); strings(x.companion_files); bool(x.companion_files_truncated);
  bool(x.structured_save_supported); nullableString(x.raw);
}
function validateSkills(method: string, x: Record<string, unknown>): void {
  if (method === 'skills.list') {
    nullableString(x.project_workdir); cursorPage(x, 'skills', validateSkillRow, 1000);
    list(x.diagnostics, d => { object(d); string(d.path); string(d.error); }); bool(x.diagnostics_truncated);
  } else if (method === 'skills.get') validateSkillDetail(x);
  else if (method === 'skills.read_file') { string(x.path); string(x.text); int(x.bytes); }
  else if (method === 'skills.remove') {
    if (x.removed !== true) throw new SdkError('protocol', 'Invalid remove result');
    if (x.now_effective !== null) validateSkillRow(x.now_effective);
    validatePropagation(x.propagation);
  } else {
    validateSkillDetail(x.skill); validatePropagation(x.propagation);
    if (method === 'skills.update') bool(x.renamed);
  }
}
function validateRoomInfo(x: unknown): void {
  object(x); string(x.session_id, true); fields(x, ['parent_session_id', 'parent_name', 'name', 'workdir']);
  bool(x.live); bool(x.locked); bool(x.bound); nullableInt(x.created_at); nullableInt(x.updated_at);
}
function validateRoomLine(x: unknown): void {
  object(x);
  if (!Number.isSafeInteger(x.line_seq) || (x.line_seq as number) < 1) throw new SdkError('protocol', 'Invalid line_seq');
  string(x.kind, true); string(x.author, true);
  fields(x, ['display', 'alias', 'lane', 'audience', 'text', 'call_id', 'tool', 'args', 'result']);
  nullableBool(x.ok); bool(x.replay); string(x.origin, true); int(x.received_at_ms); bool(x.truncated);
}
function validateCounts(x: unknown): void { object(x); int(x.open); int(x.verified); int(x.returned); int(x.reports); }
function validateRoomStatus(x: unknown): void {
  object(x); string(x.session_id, true); bool(x.live); bool(x.fresh); nullableInt(x.age_ms); int(x.now_at_ms); validateCounts(x.counts);
  string(x.counts_basis, true); strings(x.paused); bool(x.paused_truncated);
  list(x.lanes, l => {
    object(l); string(l.avatar, true); string(l.display); string(l.alias); string(l.role); nullableString(l.lane); string(l.phase, true); string(l.task);
    int(l.started_at_ms); int(l.last_event_at_ms); int(l.steps); string(l.last_tool); string(l.error);
  });
  bool(x.lanes_truncated);
  list(x.events, e => { object(e); int(e.at_ms); string(e.avatar); string(e.marker); string(e.label); });
  string(x.digest); string(x.timeline); string(x.trajectory_outline); string(x.shelf_fingerprint); bool(x.truncated);
}
function validateAttach(x: Record<string, unknown>): void {
  validateRoomInfo(x.room); bool(x.started); object(x.daemon); nullableString(x.daemon.version); strings(x.daemon.features); nullableString(x.daemon.template_revision);
  string(x.epoch, true); object(x.history); nullableInt(x.history.first_line_seq); nullableInt(x.history.last_line_seq); int(x.history.count); int(x.history.replayed);
  list(x.tail, validateRoomLine); if (x.status !== null) validateRoomStatus(x.status); strings(x.roster, true); bool(x.roster_truncated);
}
function validateShelfRow(x: unknown): void {
  object(x); string(x.key, true); int(x.revision); int(x.updated_at); string(x.kind); string(x.title); string(x.status);
}
function validateGoal(x: unknown): void {
  object(x); string(x.goal_key, true); validateCounts(x.counts); int(x.columns); int(x.rows);
  list(x.nodes, n => {
    object(n); string(n.id, true); string(n.label); string(n.kind); fields(n, ['status', 'severity', 'note', 'resource_key']); nullableInt(n.revision); int(n.rank); int(n.row);
  });
  bool(x.nodes_truncated);
  list(x.edges, e => { object(e); string(e.from, true); string(e.to, true); nullableString(e.rel); });
  bool(x.edges_truncated);
}
function validateReport(x: unknown): void {
  object(x); string(x.key, true); int(x.revision); int(x.updated_at); string(x.title); string(x.audience); string(x.severity_summary); string(x.go_no_go);
  strings(x.finding_keys); bool(x.finding_keys_truncated); nullableString(x.markdown);
}
function validateSpendEntry(x: unknown): void { object(x); int(x.calls); int(x.prompt_tokens); int(x.completion_tokens); num(x.cost); }
function validateSpend(x: Record<string, unknown>): void {
  list(x.goals, g => {
    object(g); string(g.goal_key, true); validateSpendEntry(g.total);
    list(g.by_avatar, a => { object(a); string(a.avatar, true); validateSpendEntry(a); }); bool(g.by_avatar_truncated);
  });
  bool(x.goals_truncated); object(x.ledger); string(x.ledger.ledger, true); validateTotals(x.ledger.totals);
  list(x.ledger.by_avatar, a => { object(a); string(a.avatar, true); validateTotals(a.totals); }); bool(x.ledger.by_avatar_truncated);
}
function validateField(x: unknown): void { object(x); nullableString(x.value); string(x.origin, true); }
function validateRoleLane(x: unknown): void { object(x); fields(x, ['provider_uuid', 'model', 'effort']); }
export function validateTeam(x: unknown): void {
  object(x); string(x.target, true); string(x.revision, true); bool(x.layer_present); string(x.template_revision, true); bool(x.template_pending_restart);
  list(x.avatars, a => {
    object(a); string(a.id, true); string(a.role, true); string(a.defined_in, true); bool(a.removable);
    validateField(a.display); string(a.display_effective); validateField(a.alias); string(a.alias_effective);
    list(a.lanes, l => { object(l); string(l.lane, true); validateField(l.provider_uuid); validateField(l.model); validateField(l.effort); nullableString(l.model_label); bool(l.resolves); });
  });
  bool(x.avatars_truncated);
  list(x.roles, r => {
    object(r); string(r.id, true); string(r.display); strings(r.shelf_allow); if (r.skills_allow !== null) strings(r.skills_allow);
    validateRoleLane(r.chat); validateRoleLane(r.work); string(r.defined_in, true);
  });
  bool(x.roles_truncated);
  list(x.choices, c => { object(c); string(c.id, true); string(c.name); string(c.model_id); nullableString(c.provider_name); nullableString(c.provider_uuid); });
  bool(x.choices_truncated); string(x.choices_source, true);
  list(x.warnings, w => { object(w); string(w.layer, true); string(w.path); string(w.message); }); bool(x.warnings_truncated);
}
function validateTeamWrite(x: Record<string, unknown>, method: string, params: Record<string, unknown>): void {
  validateTeam(x.team); string(x.applies, true); validatePropagation(x.propagation);
  if (method === 'room.avatars.remove') nullableString(x.reverted_to);
  if ((x.team as Record<string, unknown>).target !== params.target) throw new SdkError('protocol', 'Team target does not match the call');
}
function validateRoom(method: string, x: Record<string, unknown>, params: Record<string, unknown>): void {
  switch (method) {
    case 'room.list': cursorPage(x, 'rooms', validateRoomInfo, 500); return;
    case 'room.open': case 'room.attach': validateAttach(x); return;
    case 'room.close': if (x.closed !== true) throw new SdkError('protocol', 'Invalid close result'); bool(x.stopped); return;
    case 'room.send':
      if (x.sent !== true) throw new SdkError('protocol', 'Invalid send result');
      bool(x.acked); strings(x.inserts); int(x.echo_line_seq); return;
    case 'room.interrupt': case 'room.control': if (x.sent !== true) throw new SdkError('protocol', 'Invalid send result'); return;
    case 'room.status': validateRoomStatus(x); return;
    case 'room.history':
      string(x.epoch, true); nullableInt(x.first_line_seq); nullableInt(x.last_line_seq); list(x.lines, validateRoomLine);
      bool(x.has_more_before); bool(x.has_more_after);
      if (x.epoch !== params.epoch) throw new SdkError('protocol', 'History epoch does not match the call');
      return;
    case 'room.shelf.list': cursorPage(x, 'rows', validateShelfRow, 500); int(x.total); string(x.fingerprint); return;
    case 'room.item': validateShelfRow(x); string(x.content); if (x.json !== null) object(x.json); bool(x.truncated); return;
    case 'room.trajectory': list(x.goals, validateGoal); bool(x.goals_truncated); return;
    case 'room.reports': cursorPage(x, 'reports', validateReport, 100); return;
    case 'room.spend': validateSpend(x); return;
    case 'room.team.get': validateTeam(x); return;
    case 'room.models.set': case 'room.avatars.upsert': case 'room.avatars.remove': validateTeamWrite(x, method, params); return;
    default: throw new SdkError('protocol', `No result validator for ${method}`);
  }
}
export function validateResult(method: Method, result: unknown, params: Record<string, unknown>): void {
  object(result);
  if (method === 'configure') { validateConfigure(result); return; }
  if (method === 'create' || method === 'resume' || method === 'state') {
    validateState(result.state); optionalInfo(result);
    if (result.warnings !== undefined) list(result.warnings, w => { object(w); string(w.code, true); string(w.message); });
    return;
  }
  if (method === 'detach') { if (result.detached !== true) throw new SdkError('protocol', 'Invalid detach result'); return; }
  if (method === 'approve') { bool(result.approved); if (result.approved !== params.approve) throw new SdkError('protocol', 'Approval decision mismatch'); return; }
  if (method === 'config.get') {
    validateConfigView(result);
    if (params.view !== undefined && result.view !== params.view) throw new SdkError('protocol', 'Config view does not match the call');
    return;
  }
  if (method === 'config.providers.delete') { validateDelete(result, params, 'provider_uuid'); return; }
  if (method === 'config.models.delete') { validateDelete(result, params, 'model_uuid'); return; }
  if (method === 'config.roles.unassign') {
    bool(result.changed); nullableString(result.previous_model_uuid);
    validateMutation(result, params.scope ?? 'global'); return;
  }
  if (method === 'config.tool_policy.set') {
    bool(result.changed); strings(result.deny, true); validateMutation(result, 'global'); return;
  }
  if (method === 'config.subagent_risky.set') {
    bool(result.changed); string(result.mode, true); validateMutation(result, 'global'); return;
  }
  if (method === 'config.skill_roots.set') {
    bool(result.changed); strings(result.roots, true); validateMutation(result, 'global'); return;
  }
  if (method === 'config.effort.get') {
    string(result.state, true); strings(result.options); string(result.selected, true);
    nullableString(result.note); nullableString(result.main_model_uuid); return;
  }
  if (method === 'config.effort.set') {
    string(result.effort, true); nullableString(result.main_model_uuid); bool(result.superseded); optionalInfo(result); return;
  }
  if (method === 'config.limits.get' || method === 'config.limits.set') { validateLimits(result); optionalInfo(result); return; }
  if (method === 'sessions.info') { validateSessionInfo(result.session_info); return; }
  if (method === 'sessions.list') { cursorPage(result, 'sessions', validateSessionRow, 500); return; }
  if (method === 'sessions.rename') {
    validateSessionRow(result.session);
    if ((result.session as Record<string, unknown>).session_id !== params.session_id) throw new SdkError('protocol', 'Renamed session does not match the call');
    return;
  }
  if (method === 'sessions.delete') {
    if (result.deleted !== true) throw new SdkError('protocol', 'Invalid delete result');
    string(result.kind, true); nullableString(result.orphaned_room_session_id); return;
  }
  if (method === 'sessions.stop') { if (result.stopped !== true) throw new SdkError('protocol', 'Invalid stop result'); bool(result.was_live); return; }
  if (method === 'sessions.transcript') { validateTranscript(result, params); return; }
  if (method === 'usage.query') { validateUsage(result); return; }
  if ((SKILL_METHODS as readonly string[]).includes(method)) { validateSkills(method, result); return; }
  if ((ROOM_METHODS as readonly string[]).includes(method)) { validateRoom(method, result, params); return; }
  if (method === 'submit' || method === 'enqueue' || method === 'interrupt' || method === 'interrupt_and_submit') { validateTurnResult(method, result, params); return; }
  throw new SdkError('protocol', `No result validator for ${method}`);
}
function validateTurnResult(method: 'submit' | 'enqueue' | 'interrupt' | 'interrupt_and_submit', result: Record<string, unknown>, params: Record<string, unknown>): void {
  const success = method === 'enqueue' ? 'queued' : method === 'interrupt' ? 'interrupted' : 'started';
  member(result.disposition, [success, 'rejected']); nullableString(result.turn_id);
  if (result.disposition === 'rejected') { object(result.reason); string(result.reason.code, true); string(result.reason.message); }
  else {
    if (result.reason !== null) throw new SdkError('protocol', 'Success reason must be null');
    if (success === 'queued') { if (result.turn_id !== null) throw new SdkError('protocol', 'Queued input cannot reserve a turn'); }
    else string(result.turn_id, true);
  }
  if (method === 'interrupt' && result.turn_id !== params.turn_id) throw new SdkError('protocol', 'Interrupt target mismatch');
  if (method === 'interrupt_and_submit') {
    nullableString(result.interrupted_turn_id);
    if (result.interrupted_turn_id !== null && result.interrupted_turn_id !== params.turn_id) throw new SdkError('protocol', 'Replacement interruption target mismatch');
    if (result.disposition === 'started' && (result.interrupted_turn_id !== params.turn_id || result.turn_id === params.turn_id)) throw new SdkError('protocol', 'Replacement must start a new turn after interrupting target');
  }
}
export function local<T>(check: () => T): T {
  try { return check(); } catch (e) { throw new SdkError('invalid_input', e instanceof Error ? e.message : String(e)); }
}
export function textId(x: unknown): asserts x is string { local(() => string(x, true)); }
export function validateInput(x: Input): Record<string, unknown> {
  return local(() => {
    object(x); string(x.text, true);
    if (!x.text.trim() || Buffer.byteLength(x.text) > 2 * 1024 * 1024) throw new Error('Prompt must be nonblank and at most 2 MiB UTF-8');
    const out: Record<string, unknown> = { text: x.text };
    if (x.attachments !== undefined) {
      if (!Array.isArray(x.attachments) || x.attachments.length > 16) throw new Error('At most 16 attachments');
      let total = 0;
      out.attachments = x.attachments.map(a => {
        object(a);
        if (a.kind === 'pasted_text') {
          string(a.text); const n = Buffer.byteLength(a.text); total += n;
          if (n > 2 * 1024 * 1024) throw new Error('Pasted text exceeds 2 MiB UTF-8');
          if (a.name !== undefined && a.name !== null) string(a.name);
          return { kind: a.kind, text: a.text, ...(a.name !== undefined ? { name: a.name } : {}) };
        }
        if (a.kind === 'image_file') {
          string(a.path, true); if (!isAbsolute(a.path)) throw new Error('Image path must be absolute on daemon host');
          return { kind: a.kind, path: a.path };
        }
        throw new Error('Unsupported attachment kind');
      });
      if (total > 32 * 1024 * 1024) throw new Error('Pasted attachments exceed 32 MiB');
    }
    return out;
  });
}
