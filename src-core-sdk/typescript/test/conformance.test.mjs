import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { Client, DeliveryUnknownError, TurnReplacedError, SdkError } from '../dist/index.js';
const fake = fileURLToPath(new URL('../../conformance/fake_adapter.py', import.meta.url));
const python = process.env.PYTHON ?? 'python3';
function client(scenario='normal', extra={}) { return new Client({ executable: python, adapterArgs: [fake,'--scenario',scenario], timeoutMs:1000,...extra }); }

test('probe create resume results without event consumer', async () => {
 const c=client(); try {
  assert.equal((await c.probe()).protocol,1);
  const s=await c.create({cwd:'/fake/work'});
  assert.deepEqual((await s.getState()).policy,{plan:'forbid',approval:'reject'});
  const first=await s.submit({text:'hello é雪',attachments:[{kind:'pasted_text',text:'context'}]});
  assert.equal(first.disposition,'started');
  assert.equal((await s.enqueue({text:'next'})).disposition,'queued');
  assert.equal((await s.interrupt('stale')).disposition,'rejected');
  const replaced=await s.interruptAndSubmit(first.turn_id,{text:'replacement'});
  assert.equal(replaced.interrupted_turn_id,first.turn_id);
  await s.interrupt(replaced.turn_id);
  assert.equal((await s.waitTurn(replaced.turn_id)).state,'interrupted');
  await s.detach();
  const resumed=await c.resume({session_id:'saved'});assert.equal(resumed.sessionId,'saved');
 } finally {await c.close();}
});
test('replacement partial failure retains old interrupted turn',async()=>{
 const c=client('partial');try {const s=await c.create({cwd:'/fake/work'});const first=await s.submit({text:'hello'});const r=await s.interruptAndSubmit(first.turn_id,{text:'new'});assert.equal(r.disposition,'rejected');assert.equal(r.interrupted_turn_id,first.turn_id);}finally{await c.close();}
});
test('disconnect and explicit unknown after mutation are typed',async()=>{
 for(const scenario of ['eof','unknown','bad_result']){const c=client(scenario);try{const s=await c.create({cwd:'/fake/work'});await assert.rejects(s.submit({text:'hello'}),e=>e instanceof DeliveryUnknownError&&e.sessionId===s.sessionId&&!!e.requestId);}finally{await c.close();}}
});
test('version malformed missing capability and executable errors',async()=>{
 for(const scenario of ['bad_version','missing_cap','malformed']){const c=client(scenario);try{await assert.rejects(scenario==='malformed'?c.create({cwd:'/fake/work'}):c.probe(),SdkError);}finally{await c.close();}}
 const c=new Client({executable:'/not/an/executable'});try{await assert.rejects(c.probe(),SdkError);}finally{await c.close();}
});
test('replacement-aware Unicode output and early event before result',async()=>{
 const c=client('output');try{const s=await c.create({cwd:'/fake/work'});await s.submit({text:'hello'});const it=s.events();assert.equal((await it.next()).value.event,'state');const append=(await it.next()).value;assert.equal(append.event,'output_append');assert.equal(append.text,'é雪');await it.return();assert.equal((await s.getState()).streaming,'é雪');}finally{await c.close();}
});
test('external approvals use actual pending call id',async()=>{
 const c=client('approval');try{const s=await c.create({cwd:'/fake/work',external_approvals:true,policy:{plan:'forbid',approval:'default'}});await s.submit({text:'hello'});const a=(await s.getState()).pending_approval;await assert.rejects(s.approve('stale',true));assert.equal((await s.approve(a.call_id,true)).approved,true);}finally{await c.close();}
});
test('observation unavailable timeout cancellation stderr Unicode',async()=>{
 const c=client('hidden');try{const s=await c.create({cwd:'/fake/work'});const r=await s.submit({text:'hello'});await assert.rejects(s.waitTurn(r.turn_id),TurnReplacedError);}finally{await c.close();}
 for(const cancel of [false,true]){const c=client('timeout');try{const s=await c.create({cwd:'/fake/work'});const controller=new AbortController();const p=s.submit({text:'hello'},{timeoutMs:50,signal:controller.signal});if(cancel){await new Promise(r=>setTimeout(r,20));controller.abort();}await assert.rejects(p,DeliveryUnknownError);}finally{await c.close();}}
 const noisy=client('stderr');try{const s=await noisy.create({cwd:'/fake/work'});assert.ok(Buffer.byteLength(s.stderrTail)<=65536);assert.ok(s.stderrTail.length);}finally{await noisy.close();}
 const unicode=client('fragmented');try{const s=await unicode.create({cwd:'/fake/work'});await s.submit({text:'é雪'});assert.equal((await s.getState()).messages[0].content,'é雪');}finally{await unicode.close();}
});
test('abandoned event consumer fails explicitly instead of deadlock',async()=>{
 const c=client('event_flood');try{await assert.rejects(async()=>{const s=await c.create({cwd:'/fake/work'});await s.getState();await new Promise(r=>setTimeout(r,50));await s.events().next();},e=>e.code==='event_overflow'&&!(e instanceof DeliveryUnknownError));}finally{await c.close();}
});

test('sequence gap and bad UTF-8 offset reconcile by readback',async()=>{
 for(const scenario of ['output_gap','bad_offset']){const c=client(scenario);try{const s=await c.create({cwd:'/fake/work'});await s.submit({text:'hello'});const it=s.events();await it.next();const recovered=(await it.next()).value;assert.equal(recovered.event,'state');assert.equal(recovered.resynced,true);assert.equal(recovered.state.streaming,'é雪');await it.return();}finally{await c.close();}}
});
test('out of order and unknown responses preserve correlation',async()=>{
 const {Transport}=await import('../dist/transport.js');
 const t=new Transport({executable:python,adapterArgs:[fake,'--scenario','out_of_order'],timeoutMs:1000});
 try{const [a,b]=await Promise.all([t.request('create',{cwd:'/fake/work'}),t.request('state',{})]);assert.equal(a.state.session_id,b.state.session_id);}finally{await t.close();}
 const c=client('wrong_id');try{const s=await c.create({cwd:'/fake/work'});assert.equal((await s.submit({text:'hello'})).disposition,'started');}finally{await c.close();}
});

test('polling survives flood, restart resyncs, simultaneous consumer stays exclusive',async()=>{
 const c=client('flood_finish');try{
  const s=await c.create({cwd:'/fake/work'});
  const first=await s.submit({text:'long output'});
  assert.equal((await s.waitTurn(first.turn_id)).state,'finished');
  assert.equal(s.transport.queueBytes,0);
  await assert.rejects(s.events().next(),e=>e.code==='event_overflow');
  const it=s.events();const recovered=(await it.next()).value;
  assert.equal(recovered.resynced,true);assert.equal(recovered.state.turn.state,'finished');
  await assert.rejects(s.events().next(),e=>e.code==='invalid_state');
  const second=await s.submit({text:'again'});
  assert.equal((await s.waitTurn(second.turn_id)).state,'finished');
  await assert.rejects(it.next(),e=>e.code==='event_overflow');
  const restarted=s.events();assert.equal((await restarted.next()).value.resynced,true);await restarted.return();
 }finally{await c.close();}
});
test('overflow during recovery remains explicit and control remains usable',async()=>{
 const c=client('event_flood');try{
  const s=await c.create({cwd:'/fake/work'});await s.getState();
  for(let i=0;i<3;i++) await assert.rejects(s.events().next(),e=>e.code==='event_overflow');
  assert.equal(s.transport.queueBytes,0);assert.equal(s.transport.failure,undefined);
  assert.equal((await s.submit({text:'still usable'})).disposition,'started');
 }finally{await c.close();}
});
test('byte overflow releases accounting and validates later frames',async()=>{
 const c=client();try{
  const s=await c.create({cwd:'/fake/work'});const t=s.transport;
  const state=await s.getState();
  t.frame({type:'event',event:'state',seq:1,state,resynced:false},33*1024*1024);
  t.frame({type:'event',event:'state',seq:2,state,resynced:false},33*1024*1024);
  assert.equal(t.queueBytes,0);assert.equal(t.queue.length,0);
  await assert.rejects(s.events().next(),e=>e.code==='event_overflow');
  const it=s.events();assert.equal((await it.next()).value.resynced,true);await it.return();
  t.data(Buffer.from(JSON.stringify({type:'event',event:'output_append',seq:3,revision:0,offset:0,text:'x'})+'\n'));
  await assert.rejects(s.getState(),e=>e.code==='protocol');
 }finally{await c.close();}
});

test('recovery discards superseded appends and accepts subsequent output',async()=>{
 const c=client('flood_output');try{
  const s=await c.create({cwd:'/fake/work'});const first=await s.submit({text:'hello'});
  await assert.rejects(s.events().next(),e=>e.code==='event_overflow');
  const it=s.events();assert.equal((await it.next()).value.state.streaming,'é雪');assert.equal(s.transport.queue.length,0);
  await s.interrupt(first.turn_id);await it.next();
  await s.submit({text:'next'});await it.next();const append=(await it.next()).value;
  assert.equal(append.event,'output_append');assert.equal(append.text,'é雪');await it.return();
 }finally{await c.close();}
});

test('failed recovery retries and preserves newer observations in sequence order',async()=>{
 const c=client('flood_output');try{
  const s=await c.create({cwd:'/fake/work'});await s.submit({text:'hello'});
  await assert.rejects(s.events().next(),e=>e.code==='event_overflow');
  const current=await s.getState();let calls=0;
  s.getState=async()=>{
   if(++calls===1) throw new SdkError('timeout','readback timed out');
   s.transport.frame({type:'event',event:'output_append',seq:s.transport.seq+1,revision:current.revision+1,offset:Buffer.byteLength(current.streaming),text:'!'},100);
   return current;
  };
  await assert.rejects(s.events().next(),e=>e.code==='timeout');
  const it=s.events();const recovered=(await it.next()).value;assert.equal(recovered.resynced,true);
  const append=(await it.next()).value;assert.ok(append.seq>recovered.seq);assert.equal(append.text,'!');assert.equal(s.state.streaming,'é雪!');await it.return();
 }finally{await c.close();}
});

test('configure registers providers and swaps main without returning keys', async () => {
 const c = client();
 try {
  const configured = await c.configure({
   providers: [
    { name: 'openai', endpoint: 'https://api.openai.com/v1', api_type: 'openai', api_key: 'sk-test' },
    { name: 'anthropic', endpoint: 'https://api.anthropic.com', api_type: 'anthropic', api_key: 'sk-ant' },
   ],
   models: {
    main: { provider: 'openai', model_id: 'gpt-4.1' },
    awareness: { provider: 'openai', model_id: 'gpt-4.1-mini' },
    safeguard: { provider: 'anthropic', model_id: 'claude-sonnet-4-5' },
    compactor: { provider: 'openai', model_id: 'gpt-4.1-mini' },
    planner: { provider: 'anthropic', model_id: 'claude-sonnet-4-5' },
   },
  });
  assert.equal(configured.providers[0].uuid, 'prov-openai');
  assert.equal(configured.providers[1].name, 'anthropic');
  assert.equal(configured.models.safeguard.provider, 'anthropic');
  assert.equal(JSON.stringify(configured).includes('api_key'), false);
  assert.equal(JSON.stringify(configured).includes('sk-test'), false);
  const session = await c.create({ cwd: '/fake/work' });
  const swapped = await session.configure({ models: { main: { provider: 'anthropic', model_id: 'claude-sonnet-4-5' } } });
  assert.deepEqual(swapped.providers, []);
  assert.equal(swapped.models.main.model_id, 'claude-sonnet-4-5');
  assert.equal(JSON.stringify(swapped).includes('sk-ant'), false);
 } finally { await c.close(); }
});

test('configure refuses an empty key before sending', async () => {
 const log = join(tmpdir(), `koma-configure-${process.pid}.log`);
 rmSync(log, { force: true });
 const c = client('normal', { env: { KOMA_FAKE_LOG: log } });
 try {
  await assert.rejects(
   c.configure({ providers: [{ name: 'openai', endpoint: 'https://api.openai.com/v1', api_type: 'openai', api_key: '  ' }] }),
   e => e instanceof SdkError && e.code === 'invalid_input',
  );
  if (existsSync(log)) assert.equal(readFileSync(log, 'utf8').includes('configure'), false);
 } finally { await c.close(); rmSync(log, { force: true }); }
});

const SECRETS = ['sk-fake-planted', 'fake-pass', 'fake-query', 'fake-oauth-token'];
function noSecrets(value) { const text = JSON.stringify(value); for (const s of SECRETS) assert.equal(text.includes(s), false, s); }
// An adapter that sends a current hello and never answers, so every request ends locally.
const SILENT = `import json,sys
v1=['create','resume','state','submit','enqueue','interrupt','interrupt_and_submit','approve','configure','detach']
print(json.dumps(dict(type='hello',protocol=1,binary=dict(path='/silent',version='0'),diagnostics=[],capabilities=v1,
 methods=v1+['config.get','config.providers.delete','config.models.delete','config.roles.unassign','config.skill_roots.set'],features=[],events=[],limits={})),flush=True)
for _ in sys.stdin: pass`;

test('hello carries methods features events limits and requires only the v1 capabilities', async () => {
 const c = client('config');
 try {
  const hello = await c.probe();
  assert.deepEqual(hello.capabilities, ['create','resume','state','submit','enqueue','interrupt','interrupt_and_submit','approve','configure','detach']);
  assert.ok(hello.methods.includes('config.get') && hello.methods.includes('sessions.info'));
  assert.ok(hello.features.includes('errors.reason'));
  assert.deepEqual(hello.events.slice(0, 3), ['state', 'output_append', 'turn']);
  assert.ok(hello.events.includes('room.line') && hello.events.includes('room.gap'));
  assert.equal(hello.limits.request_ids, 65536);
 } finally { await c.close(); }
 const legacy = client('legacy_hello');
 try {
  const hello = await legacy.probe();
  assert.deepEqual([hello.methods, hello.features, hello.events, hello.limits], [[], [], [], {}]);
 } finally { await legacy.close(); }
});

test('host reads a redacted config view without secrets', async () => {
 const c = client('config'); const host = c.host();
 try {
  assert.equal(await host.supports('config.get'), true);
  const view = await host.getConfig();
  assert.equal(view.view, 'global'); assert.equal(view.file, 'ok'); assert.match(view.revision, /^[0-9a-f]{16}$/);
  const local = view.providers.find(p => p.provider_uuid === 'prov-local');
  assert.equal(local.endpoint, 'https://llm.example.test/v1'); assert.equal(local.endpoint_redacted, true);
  assert.equal(local.key_state, 'set'); assert.equal(local.has_key, true);
  assert.equal(view.models.find(m => m.model_uuid === 'model-planner').other_roles, true);
  assert.equal(view.models.find(m => m.model_uuid === 'model-orphan').provider_kind, 'missing');
  assert.equal(view.roles.length, 5);
  noSecrets(view);
 } finally { await host.close(); await c.close(); }
});

test('host deletes: dry run, stale revision, managed, not found, committed', async () => {
 const c = client('config'); const host = c.host();
 try {
  const before = await host.getConfig();
  const dry = await host.deleteProvider({ provider_uuid: 'prov-local', dry_run: true });
  assert.equal(dry.dry_run, true); assert.deepEqual(dry.removed.models_removed, ['model-main', 'model-planner']);
  assert.equal(dry.removed.main_reset, true); assert.deepEqual(dry.propagation, {});
  assert.equal((await host.getConfig()).revision, before.revision);
  await assert.rejects(host.deleteProvider({ provider_uuid: 'prov-local', if_revision: '0000000000000000' }),
   e => e instanceof SdkError && !(e instanceof DeliveryUnknownError) && e.reason === 'stale' && e.code === 'invalid_state' && e.delivery === 'not_sent');
  await assert.rejects(host.deleteProvider({ provider_uuid: 'prov-ext' }), e => e.reason === 'managed');
  await assert.rejects(host.deleteModel({ model_uuid: 'model-none' }), e => e.code === 'not_found' && e.reason === 'not_found');
  await assert.rejects(host.deleteModel({ model_uuid: '' }), e => e.code === 'invalid_input');
  const done = await host.deleteModel({ model_uuid: 'model-orphan', if_revision: before.revision });
  assert.equal(done.dry_run, false); assert.equal(done.config.view, 'global');
  assert.equal(done.propagation.bound, 'not_bound'); assert.equal(done.propagation.rooms, 'next_start');
  assert.equal(done.config.models.some(m => m.model_uuid === 'model-orphan'), false);
  assert.notEqual(done.config.revision, before.revision);
  const unchanged = await host.unassignRole({ role: 'compactor' });
  assert.equal(unchanged.changed, false);
  const released = await host.unassignRole({ role: 'main' });
  assert.equal(released.changed, true); assert.equal(released.previous_model_uuid, 'model-main');
  assert.equal(released.config.main_fallback, 'unconfigured');
  noSecrets([dry, done, released]);
 } finally { await host.close(); await c.close(); }
});

test('setSkillRoots replaces the external skill roots on a host and on a session', async () => {
 const dir = realpathSync(mkdtempSync(join(tmpdir(), 'koma-roots-')));
 const c = client('config'); const host = c.host();
 try {
  const before = await host.getConfig();
  const first = await host.setSkillRoots({ roots: [dir] });
  assert.equal(first.changed, true); assert.deepEqual(first.roots, [dir]); assert.equal(first.config.view, 'global');
  assert.equal(first.propagation.bound, 'not_bound'); assert.notEqual(first.config.revision, before.revision);
  const again = await host.setSkillRoots({ roots: [dir], if_revision: first.config.revision });
  assert.equal(again.changed, false);
  await assert.rejects(host.setSkillRoots({ roots: [dir], if_revision: '0000000000000000' }), e => e.reason === 'stale');
  await assert.rejects(host.setSkillRoots({ roots: [join(dir, 'absent')] }), e => e.reason === 'out_of_range');
  await assert.rejects(host.setSkillRoots({ roots: dir }), e => e.code === 'invalid_input');
  await assert.rejects(host.setSkillRoots({ roots: [''] }), e => e.code === 'invalid_input');
  const cleared = await host.setSkillRoots({ roots: [] });
  assert.equal(cleared.changed, true); assert.deepEqual(cleared.roots, []);
  noSecrets([first, again, cleared]);
  const s = await c.create({ cwd: '/fake/work' });
  const set = await s.setSkillRoots({ roots: [dir] });
  assert.equal(set.changed, true); assert.equal(set.propagation.bound, 'reloaded'); assert.ok(set.session_info);
 } finally { await host.close(); await c.close(); rmSync(dir, { recursive: true, force: true }); }
});

test('an unverified delete is a lost mutation with written and detail; the host stays usable', async () => {
 const c = client('config'); const host = c.host();
 try {
  await assert.rejects(host.deleteModel({ model_uuid: 'model-unverified' }), e => e instanceof DeliveryUnknownError
   && e.reason === 'unverified' && e.written === true && e.code === 'unavailable' && e.method === 'config.models.delete'
   && Array.isArray(e.detail.diff) && e.detail.diff[0] === 'model-unverified');
  assert.equal((await host.getConfig()).models.some(m => m.model_uuid === 'model-unverified'), false);
 } finally { await host.close(); await c.close(); }
});

test('a lost reply is a lost mutation only for calls that write', async () => {
 const c = new Client({ executable: python, adapterArgs: ['-c', SILENT], timeoutMs: 1000 }); const host = c.host();
 try {
  await assert.rejects(host.deleteProvider({ provider_uuid: 'p', dry_run: true }, { timeoutMs: 30 }),
   e => !(e instanceof DeliveryUnknownError) && e.code === 'timeout' && e.delivery === 'unknown');
  await assert.rejects(host.getConfig({}, { timeoutMs: 30 }), e => !(e instanceof DeliveryUnknownError) && e.delivery === 'unknown');
  await assert.rejects(host.deleteProvider({ provider_uuid: 'p' }, { timeoutMs: 30 }), e => e instanceof DeliveryUnknownError);
  await assert.rejects(host.deleteModel({ model_uuid: 'm', dry_run: false }, { timeoutMs: 30 }), e => e instanceof DeliveryUnknownError);
  await assert.rejects(host.unassignRole({ role: 'main' }, { timeoutMs: 30 }), e => e instanceof DeliveryUnknownError);
  await assert.rejects(host.setSkillRoots({ roots: [] }, { timeoutMs: 30 }), e => e instanceof DeliveryUnknownError);
 } finally { await host.close(); await c.close(); }
});

test('host replaces its adapter before the request-id limit and after a failure', async () => {
 const { HOST_RECYCLE_AFTER } = await import('../dist/index.js');
 const c = client('config'); const host = c.host();
 try {
  await host.getConfig();
  const first = await host.transport();
  first.sent = HOST_RECYCLE_AFTER - 1;
  const inflight = host.getConfig();
  while (first.sent < HOST_RECYCLE_AFTER) await new Promise(r => setImmediate(r));
  const next = host.getConfig();
  assert.equal((await inflight).view, 'global'); assert.equal((await next).view, 'global');
  const second = await host.transport();
  assert.notEqual(second, first); assert.equal(second.requestsSent, 1);
  await first.exited; assert.equal(c.connections.has(first), false);
  second.child.kill('SIGKILL'); await second.exited;
  assert.equal((await host.getConfig()).view, 'global');
  assert.notEqual(await host.transport(), second);
 } finally { await host.close(); await c.close(); }
});

test('wrong binding errors expose reason binding allowed', async () => {
 const { Transport } = await import('../dist/transport.js');
 const t = new Transport({ executable: python, adapterArgs: [fake, '--scenario', 'wrong_binding'], timeoutMs: 1000 });
 try {
  await assert.rejects(t.request('state', {}), e => e.code === 'invalid_state' && e.message === 'no session bound'
   && e.reason === 'wrong_binding' && e.binding === 'unbound' && e.allowed.join() === 'coding' && e.delivery === 'not_sent');
  await assert.rejects(t.request('config.effort.get', {}), e => e.reason === 'wrong_binding' && e.allowed.join() === 'coding');
  await assert.rejects(t.request('config.get', { view: 'session' }), e => e.reason === 'wrong_binding');
  await t.request('create', { cwd: '/fake/work' });
  await assert.rejects(t.request('resume', { session_id: 'x' }), e => e.reason === 'already_bound' && e.binding === 'coding' && e.message === 'session already bound');
 } finally { await t.close(); }
 const unbound = new Transport({ executable: python, adapterArgs: [fake, '--scenario', 'wrong_binding'], timeoutMs: 1000 });
 try {
  // The role is checked before the binding.
  await assert.rejects(unbound.request('room.status', {}), e => e.reason === 'wrong_binding' && e.allowed.join() === 'room_live,room_closed');
  await assert.rejects(unbound.request('config.roles.unassign', { role: 'planner', scope: 'session' }), e => e.reason === 'out_of_range');
  // Unbound detach keeps its v1 error, with no added keys.
  await assert.rejects(unbound.request('detach', {}), e => e.code === 'invalid_state' && e.message === 'no session bound'
   && e.reason === undefined && e.binding === undefined);
 } finally { await unbound.close(); }
 const closed = new Transport({ executable: python, adapterArgs: [fake, '--scenario', 'wrong_binding', '--binding', 'room_closed'], timeoutMs: 1000 });
 try {
  await assert.rejects(closed.request('config.limits.get', {}), e => e.reason === 'wrong_binding' && e.binding === 'room_closed'
   && e.message === 'config.limits.get requires binding coding; this adapter is room_closed');
  assert.equal((await closed.request('config.get', {})).view, 'global');
 } finally { await closed.close(); }
});

test('session config calls keep session_info ordered by observed_at_ms', async () => {
 const c = client('config');
 try {
  const s = await c.create({ cwd: '/fake/work' });
  const created = s.sessionInfo;
  assert.equal(created.name, 'fake session'); assert.equal(created.effort, 'default');
  assert.equal(created.routes.find(r => r.role === 'main').origin, 'session');
  const remaining = s.requestsRemaining;
  assert.equal(remaining, 65536 - 1);
  assert.equal((await s.getConfig()).view, 'session');
  assert.equal(s.requestsRemaining, remaining - 1);
  const { revision } = await s.getState();
  const limits = await s.setLimits({ max_output_tokens: 4096 });
  assert.equal(limits.max_output_tokens, 4096); assert.equal(s.sessionInfo.limits.max_output_tokens, 4096);
  assert.ok(s.sessionInfo.observed_at_ms > created.observed_at_ms);
  // A state result is applied by observed_at_ms even when the state revision did not move.
  s.latestInfo = created;
  assert.equal((await s.getState()).revision, revision);
  assert.equal(s.sessionInfo.limits.max_output_tokens, 4096);
  s.observe({ session_info: created });
  assert.equal(s.sessionInfo.limits.max_output_tokens, 4096);
  assert.equal((await s.info()).limits.max_output_tokens, 4096);
  assert.equal((await s.getLimits()).max_output_tokens, 4096);
  await assert.rejects(s.setLimits({}), e => e.reason === 'missing_params' && e.code === 'invalid_request');
  const loading = await s.getEffort({ wait_ms: 0 });
  assert.equal(loading.state, 'loading'); assert.equal(loading.main_model_uuid, 'model-main');
  await assert.rejects(s.setEffort({ effort: 'high', if_main_model: 'model-other' }), e => e.reason === 'main_changed' && !(e instanceof DeliveryUnknownError));
  await assert.rejects(s.setEffort({ effort: 'extreme' }), e => e.reason === 'not_offered');
  const effort = await s.setEffort({ effort: 'high', if_main_model: 'model-main' });
  assert.equal(effort.superseded, false); assert.equal(s.sessionInfo.effort, 'high');
  assert.deepEqual((await s.getEffort()).options, ['default', 'low', 'medium', 'high']);
  await assert.rejects(s.unassignRole({ role: 'main', scope: 'session', if_revision: 'absent' }), e => e.reason === 'conflicting_params');
  await assert.rejects(s.unassignRole({ role: 'planner', scope: 'session' }), e => e.reason === 'out_of_range');
  const unpinned = await s.clearMainPin();
  assert.equal(unpinned.changed, true); assert.equal(unpinned.config.view, 'session');
  assert.equal(s.sessionInfo.effort, 'default'); assert.equal(s.sessionInfo.routes.find(r => r.role === 'main').origin, 'global');
  const dry = await s.deleteProvider({ provider_uuid: 'prov-local', dry_run: true });
  assert.equal(dry.dry_run, true); assert.ok(dry.session_info);
  // Without a scope a session's unassignRole is global, as on Host.
  const released = await s.unassignRole({ role: 'planner' });
  assert.equal(released.changed, true); assert.equal(released.config.view, 'global');
  assert.equal(s.sessionInfo.routes.find(r => r.role === 'main').origin, 'global');
  noSecrets([s.sessionInfo, dry, unpinned]);
 } finally { await c.close(); }
});

test('an older adapter is gated on capabilities and has no session_info or reasons', async () => {
 const log = join(tmpdir(), `koma-legacy-${process.pid}.log`);
 rmSync(log, { force: true });
 const c = client('legacy_hello', { env: { KOMA_FAKE_LOG: log } }); const host = c.host();
 try {
  assert.equal(await host.supports('config.get'), false);
  await assert.rejects(host.getConfig(), e => e.code === 'unsupported_method' && e.delivery === 'not_sent');
  const s = await c.create({ cwd: '/fake/work' });
  assert.equal(s.sessionInfo, null); assert.equal(s.supports('state'), true); assert.equal(s.supports('sessions.info'), false);
  await assert.rejects(s.info(), e => e.code === 'unsupported_method');
  await assert.rejects(s.setLimits({ max_output_tokens: 4096 }), e => e.code === 'unsupported_method' && !(e instanceof DeliveryUnknownError));
  assert.equal((await s.getState()).revision, 1); assert.equal(s.sessionInfo, null);
  assert.equal(s.requestsRemaining, 65536 - 2);
  assert.equal(readFileSync(log, 'utf8').includes('config.'), false);
 } finally { await host.close(); await c.close(); rmSync(log, { force: true }); }
 const { Transport } = await import('../dist/transport.js');
 const t = new Transport({ executable: python, adapterArgs: [fake, '--scenario', 'legacy_hello'], timeoutMs: 1000 });
 try { await assert.rejects(t.request('state', {}), e => e.code === 'invalid_state' && e.reason === undefined && e.allowed === undefined); }
 finally { await t.close(); }
});

test('new enum-like strings are open; v1 enums and public roles stay closed', async () => {
 const { validateResult } = await import('../dist/validate.js');
 const route = role => ({ role, origin: 'someday', model_uuid: null, source_uuid: null, configured_model: null, provider_uuid: null, provider_name: null, effective_model: null, reason: 'new_reason' });
 const view = { revision: 'absent', file: 'elsewhere', view: 'global', providers: [{ provider_uuid: 'p', name: 'n', api_type: 'gemini', endpoint: 'e', endpoint_redacted: false, managed: false, has_key: false, key_state: 'sealed', model_count: 0 }],
  providers_truncated: false, oauth: [], oauth_truncated: false, models: [{ model_uuid: 'm', name: 'm', model_id: 'm', provider_uuid: 'p', provider_kind: 'plugin', provider_name: null, route: null, roles: ['main'], other_roles: false, source_uuid: null }],
  models_truncated: false, roles: ['main', 'awareness', 'safeguard', 'compactor', 'planner'].map(route), main_fallback: 'future' };
 validateResult('config.get', view, {});
 validateResult('config.roles.unassign', { changed: false, previous_model_uuid: null, config: view, propagation: { bound: 'later', extra: 'x' } }, { role: 'main' });
 validateResult('config.skill_roots.set', { changed: true, roots: ['/srv/skills'], config: view, propagation: { bound: 'later' } }, { roots: ['/srv/skills'] });
 assert.throws(() => validateResult('config.skill_roots.set', { changed: true, roots: [1], config: view, propagation: {} }, {}), e => e.code === 'protocol');
 validateResult('config.effort.get', { state: 'warming', options: ['default'], selected: 'default', note: null, main_model_uuid: null }, {});
 assert.throws(() => validateResult('config.get', { ...view, models: [{ ...view.models[0], roles: ['cyber-red'] }] }, {}), e => e.code === 'protocol');
 assert.throws(() => validateResult('config.get', { ...view, providers: [{ ...view.providers[0], api_key: 'x' }] }, {}), e => e.code === 'protocol');
 assert.throws(() => validateResult('config.get', view, { view: 'session' }), e => e.code === 'protocol');
 assert.throws(() => validateResult('config.providers.delete', { dry_run: false, removed: { provider_uuid: 'p', models_removed: [], main_reset: false }, config: view, propagation: {} }, { provider_uuid: 'p', dry_run: true }), e => e.code === 'protocol');
 assert.throws(() => validateResult('sessions.info', { disposition: 'started', turn_id: 't', reason: null }, {}), e => e.code === 'protocol');
 const state = { revision: 1, external_approvals: false, pending_approval: null, gate: null, session_id: 's', cwd: '/w', model_id: 'm', policy: { plan: 'forbid', approval: 'reject' }, turn: null, working: false, awaiting_approval: false, approval_reason: null, pending_count: 0, messages: [], streaming: null };
 validateResult('state', { state }, {});
 assert.throws(() => validateResult('state', { state: { ...state, messages: [{ role: 'robot', content: '' }] } }, {}), e => e.code === 'protocol');
 assert.throws(() => validateResult('state', { state: { ...state, turn: { id: 't', state: 'paused', error: null } } }, {}), e => e.code === 'protocol');
});
