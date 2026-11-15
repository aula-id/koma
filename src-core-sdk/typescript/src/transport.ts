import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { isAbsolute } from 'node:path';
import { METHODS, SdkError, DeliveryUnknownError, type ClientOptions, type ErrorDetails, type Hello, type Method, type RequestOptions, type Event, type State } from './types.js';
import { errorDetails, implementedMethods, validateHello, validateEvent, validateResult, object, string } from './validate.js';

type Incoming = Event & { gap?: boolean };
export const MAX_FRAME_BYTES = 40 * 1024 * 1024;
const MAX_EVENTS_BYTES = 64 * 1024 * 1024;
/** Per-process request cap of adapters that do not advertise `limits.request_ids`. */
export const DEFAULT_REQUEST_IDS = 65_536;
const mutations = new Set<Method>(['create', 'resume', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve', 'configure',
  'config.providers.delete', 'config.models.delete', 'config.roles.unassign', 'config.effort.set', 'config.limits.set']);
/** A delete with `dry_run:true` writes nothing, so a lost reply to it is not a lost mutation. */
function isMutation(method: Method, params: Record<string, unknown>): boolean {
  if ((method === 'config.providers.delete' || method === 'config.models.delete') && params.dry_run === true) return false;
  return mutations.has(method);
}
type Pending = { method: Method; params: Record<string, unknown>; written: boolean; resolve: (x: unknown) => void; reject: (e: unknown) => void; cleanup: () => void };

export class Transport {
  readonly child: ChildProcessWithoutNullStreams;
  readonly ready: Promise<Hello>;
  hello?: Hello;
  private readyResolve!: (h: Hello) => void;
  private readyReject!: (e: unknown) => void;
  private pending = new Map<string, Pending>();
  private counter = 0;
  private sent = 0;
  private writeBytes = 0;
  private parts: Buffer[] = [];
  private bytes = 0;
  private seq = 0;
  private gap = false;
  private queue: { event: Incoming; bytes: number }[] = [];
  private queueBytes = 0;
  private waiter?: { resolve: (v: IteratorResult<Incoming>) => void; reject: (e: unknown) => void };
  private iterating = false;
  private failure?: SdkError;
  private eventError?: SdkError;
  private overflowObserved = false;
  private resyncRequired = false;
  private ended = false;
  private expectedExit = false;
  private stderrTail = Buffer.alloc(0);
  private exited: Promise<void>;
  private helloTimer: ReturnType<typeof setTimeout>;
  constructor(private readonly options: ClientOptions) {
    this.ready = new Promise((resolve, reject) => { this.readyResolve = resolve; this.readyReject = reject; });
    // Attach immediately: startup can fail before callers await ready.
    void this.ready.catch(() => {});
    this.child = spawn(options.executable ?? 'koma', [...(options.adapterArgs ?? ['sdk', 'serve'])], { shell: false, stdio: 'pipe', env: { ...process.env, ...options.env } });
    this.exited = new Promise(resolve => this.child.once('close', () => resolve()));
    this.helloTimer = setTimeout(() => this.fail(new SdkError('timeout', 'Timed out waiting for adapter hello')), options.timeoutMs ?? 30_000);
    this.child.stdout.on('data', (chunk: Buffer) => this.data(chunk));
    this.child.stderr.on('data', (chunk: Buffer) => {
      this.stderrTail = Buffer.concat([this.stderrTail, chunk.subarray(Math.max(0, chunk.length - 65536))]).subarray(-65536);
    });
    this.child.stdin.on('error', e => this.fail(new SdkError('transport', e.message)));
    this.child.on('error', e => this.fail(new SdkError('transport', e.message)));
    this.child.stdout.on('end', () => {
      if (!this.expectedExit) this.fail(new SdkError('transport', this.bytes ? 'Truncated JSONL frame at EOF' : 'Unexpected adapter EOF'));
    });
    this.child.on('close', (code, signal) => {
      if (!this.expectedExit) this.fail(new SdkError('transport', `Adapter exited (${code ?? signal})`));
      this.ended = true;
      this.waiter?.resolve({ done: true, value: undefined }); this.waiter = undefined;
    });
  }
  get diagnostics(): string { return this.stderrTail.toString('utf8'); }
  /** Requests written to this adapter so far. */
  get requestsSent(): number { return this.sent; }
  /** Requests this adapter accepts before the next one is fatal (hello `limits.request_ids`). */
  get requestIdLimit(): number {
    const limit = this.hello?.limits.request_ids;
    return Number.isSafeInteger(limit) && (limit as number) > 0 ? limit as number : DEFAULT_REQUEST_IDS;
  }
  get requestsRemaining(): number { return Math.max(0, this.requestIdLimit - this.sent); }
  /** False once the adapter failed, exited or was closed; a stateless caller may then start a new one. */
  get usable(): boolean { return !this.failure && !this.ended && !this.expectedExit; }
  /** Whether the adapter implements a method: hello `methods`, or `capabilities` for an older adapter. */
  supports(method: string): boolean { return !!this.hello && implementedMethods(this.hello).includes(method); }
  private data(chunk: Buffer): void {
    if (this.failure) return;
    try {
      let start = 0;
      for (let i = 0; i < chunk.length; i++) {
        if (chunk[i] !== 10) continue;
        const part = chunk.subarray(start, i);
        if (this.bytes + part.length > MAX_FRAME_BYTES) throw new SdkError('frame_too_large', 'Adapter frame exceeds 40 MiB');
        this.parts.push(part);
        const size = this.bytes + part.length;
        const line = Buffer.concat(this.parts, size);
        this.parts = []; this.bytes = 0; start = i + 1;
        // Fatal decoding handles split multibyte sequences without replacement characters.
        const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(line);
        this.frame(JSON.parse(text), size);
        if (this.failure) return;
      }
      if (start < chunk.length) {
        const tail = chunk.subarray(start);
        if (this.bytes + tail.length > MAX_FRAME_BYTES) throw new SdkError('frame_too_large', 'Adapter frame exceeds 40 MiB');
        this.parts.push(tail); this.bytes += tail.length;
      }
    } catch (e) { this.fail(e instanceof SdkError ? e : new SdkError('protocol', `Invalid adapter frame: ${String(e)}`)); }
  }
  private frame(frame: unknown, bytes: number): void {
    object(frame);
    if (!this.hello) {
      this.hello = validateHello(frame, this.options.requiredCapabilities ?? METHODS);
      clearTimeout(this.helloTimer); this.readyResolve(this.hello); return;
    }
    if (frame.type === 'event') {
      if (!Number.isSafeInteger(frame.seq) || (frame.seq as number) <= this.seq) throw new SdkError('protocol', 'Invalid event sequence');
      this.gap ||= frame.seq !== this.seq + 1;
      this.seq = frame.seq as number;
      if (!['state', 'output_append', 'turn'].includes(frame.event as string)) return; // Additive event kinds retain sequence validation.
      validateEvent(frame);
      const event = { ...frame, gap: this.gap } as unknown as Incoming; this.gap = false;
      if (this.eventError) return;
      if (this.waiter) { const w = this.waiter; this.waiter = undefined; w.resolve({ done: false, value: event }); }
      else {
        if (this.queue.length >= 256 || this.queueBytes + bytes > MAX_EVENTS_BYTES) {
          this.queue = []; this.queueBytes = 0;
          this.eventError = new SdkError('event_overflow', 'Event consumer exceeded 256 frames / 64 MiB; restart events to reconcile state');
          this.overflowObserved = false;
          return;
        }
        this.queue.push({ event, bytes }); this.queueBytes += bytes;
      }
      return;
    }
    if (frame.type !== 'response') throw new SdkError('protocol', 'Expected response or event after hello');
    string(frame.id, true);
    const hasResult = Object.hasOwn(frame, 'result'), hasError = Object.hasOwn(frame, 'error');
    if (hasResult === hasError) throw new SdkError('protocol', 'Response must contain exactly one of result/error');
    const pending = this.pending.get(frame.id as string);
    if (!pending) return; // Late, timed-out, or unknown response is never reassigned.
    if (hasError) {
      object(frame.error); string(frame.error.code, true); string(frame.error.message);
      if (frame.error.delivery !== 'unknown' && frame.error.delivery !== 'not_sent') throw new SdkError('protocol', 'Invalid error delivery');
      const details = errorDetails(frame.error);
      this.finish(frame.id as string);
      pending.reject(this.requestError(frame.error.code as string, frame.error.message as string, frame.id as string, pending, frame.error.delivery === 'unknown', details));
    } else {
      validateResult(pending.method, frame.result, pending.params);
      if (pending.method === 'detach') this.expectedExit = true;
      this.finish(frame.id as string); pending.resolve(frame.result);
    }
  }
  private requestError(code: string, message: string, id: string, p: Pending, unknown = p.written, details: ErrorDetails = {}): SdkError {
    let error: SdkError;
    if (unknown && isMutation(p.method, p.params)) {
      const lost = new DeliveryUnknownError(code, message, id, p.method);
      lost.turnId = p.params.turn_id as string | undefined;
      lost.sessionId = p.params.session_id as string | undefined;
      error = lost;
    } else error = new SdkError(code, message, unknown ? 'unknown' : 'not_sent', id, p.method);
    return Object.assign(error, details);
  }
  private finish(id: string): void { this.pending.get(id)?.cleanup(); this.pending.delete(id); }
  async request<T>(method: Method, params: Record<string, unknown>, options: RequestOptions = {}): Promise<T> {
    if (options.timeoutMs !== undefined && (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0)) throw new SdkError('invalid_input', 'timeoutMs must be positive');
    await this.ready;
    const id = String(++this.counter);
    if (this.failure) throw new SdkError(this.failure.code, this.failure.message, 'not_sent', id, method);
    if (this.expectedExit || this.ended) throw new SdkError('closed', 'Adapter is closed', 'not_sent', id, method);
    if (!this.supports(method)) throw new SdkError('unsupported_method', `Adapter does not support ${method}`, 'not_sent', id, method);
    if (this.pending.size >= 64) throw new SdkError('pending_overflow', 'At most 64 pending requests', 'not_sent', id, method);
    if (options.signal?.aborted) throw new SdkError('cancelled', 'Local await cancelled', 'not_sent', id, method);
    const line = JSON.stringify({ id, method, params });
    if (Buffer.byteLength(line) > MAX_FRAME_BYTES) throw new SdkError('frame_too_large', 'Serialized request exceeds 40 MiB', 'not_sent', id, method);
    const size = Buffer.byteLength(line) + 1;
    if (this.writeBytes + size > MAX_EVENTS_BYTES) throw new SdkError('pending_overflow', 'Outbound buffer exceeds 64 MiB');
    this.writeBytes += size;
    return new Promise<T>((resolve, reject) => {
      const cancel = (code: string) => {
        const p = this.pending.get(id); if (!p) return;
        this.finish(id); reject(this.requestError(code, code === 'timeout' ? 'Request timed out; no retry was attempted' : 'Local await cancelled; turn was not interrupted', id, p));
      };
      const timer = setTimeout(() => cancel('timeout'), options.timeoutMs ?? this.options.timeoutMs ?? 30_000);
      const abort = () => cancel('cancelled');
      const p: Pending = { method, params, written: false, resolve: x => resolve(x as T), reject, cleanup: () => { clearTimeout(timer); options.signal?.removeEventListener('abort', abort); } };
      this.pending.set(id, p);
      options.signal?.addEventListener('abort', abort, { once: true });
      // Conservatively mark before write: even a write error can follow a partial dispatch.
      p.written = true; this.sent++;
      try { this.child.stdin.write(line + '\n', e => { this.writeBytes -= size; if (e) this.fail(new SdkError('transport', e.message)); }); }
      catch (e) { this.writeBytes -= size; this.fail(new SdkError('transport', String(e))); }
    });
  }
  private fail(error: SdkError): void {
    if (this.failure) return;
    this.failure = error; clearTimeout(this.helloTimer); this.readyReject(error);
    for (const [id, p] of this.pending) { this.finish(id); p.reject(this.requestError(error.code, error.message, id, p)); }
    this.queue = []; this.queueBytes = 0; this.parts = []; this.bytes = 0;
    this.waiter?.reject(error); this.waiter = undefined;
    this.stop();
  }
  private stop(): void {
    this.child.stdin.end();
    // Only our adapter child is signalled, never a process group or the daemon.
    const term = setTimeout(() => this.child.kill('SIGTERM'), 200);
    const kill = setTimeout(() => this.child.kill('SIGKILL'), 1000);
    term.unref(); kill.unref();
    void this.exited.then(() => { clearTimeout(term); clearTimeout(kill); });
  }
  async close(): Promise<void> {
    this.expectedExit = true;
    for (const [id, p] of this.pending) { this.finish(id); p.reject(this.requestError('closed', 'Client closed', id, p)); }
    clearTimeout(this.helloTimer); this.readyReject(new SdkError('closed', 'Client closed'));
    this.stop();
    await this.exited;
  }
  private checkEvents(): void {
    if (this.failure) throw this.failure;
    if (this.eventError) { this.overflowObserved = true; throw this.eventError; }
  }
  async *events(options: Pick<RequestOptions, 'signal'> = {}, recover?: () => Promise<State>): AsyncGenerator<Incoming> {
    if (this.iterating) throw new SdkError('invalid_state', 'Only one event iterator can be active');
    this.iterating = true;
    try {
      if ((this.overflowObserved || (this.resyncRequired && !this.eventError)) && recover) {
        this.resyncRequired = true;
        this.eventError = undefined; this.overflowObserved = false;
        const seq = this.seq;
        const state = await recover();
        this.checkEvents();
        this.queue = this.queue.filter(({ event }) => (event.event === 'state' ? event.state.revision : event.revision) > state.revision);
        this.queueBytes = this.queue.reduce((sum, item) => sum + item.bytes, 0);
        this.resyncRequired = false;
        yield { type: 'event', event: 'state', seq, state, resynced: true };
      }
      while (true) {
        if (options.signal?.aborted) throw new SdkError('cancelled', 'Event await cancelled; turn was not interrupted');
        this.checkEvents();
        const item = this.queue.shift();
        if (item) { this.queueBytes -= item.bytes; yield item.event; continue; }
        if (this.ended) return;
        const next = await new Promise<IteratorResult<Incoming>>((resolve, reject) => {
          const abort = () => { this.waiter = undefined; reject(new SdkError('cancelled', 'Event await cancelled; turn was not interrupted')); };
          this.waiter = { resolve: value => { options.signal?.removeEventListener('abort', abort); resolve(value); }, reject: e => { options.signal?.removeEventListener('abort', abort); reject(e); } };
          options.signal?.addEventListener('abort', abort, { once: true });
        });
        if (next.done) return;
        yield next.value;
      }
    } finally { this.iterating = false; }
  }
}
