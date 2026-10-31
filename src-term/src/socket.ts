import { createConnection, type Socket } from "node:net"
import type { DaemonTransport } from "./transport.ts"
import { EventDispatcher, PendingOperations } from "./events.ts"
import { controlOf, encodeFrame, fold, FrameReader, isSnapshotEvent, needsRestart, needsResync, type Control, type Snapshot } from "./protocol.ts"
import { ensureDaemon, killSession, socketPath } from "./paths.ts"
import { countFrames } from "./memlog.ts"

type ClientSocket = {
  write(data: Uint8Array): number | boolean
  end(): void
}

export class DaemonLink {
  private socket: ClientSocket | null = null
  private reader = new FrameReader()
  private seq = -1
  /** A Resync is in flight; deltas until the next Snapshot would apply to a stale shadow. */
  private resyncing = false
  private closed = false
  private detached = false
  private restarting = false
  private helloResolve: ((version: string | null) => void) | null = null
  readonly events = new EventDispatcher()
  readonly operations = new PendingOperations()
  menuVersion = 0

  snap: Snapshot | null = null

  constructor(
    private readonly onUpdate: (snap: Snapshot | null) => void,
    private readonly onControl: (control: Control) => void,
    private readonly transport?: DaemonTransport,
  ) {
    this.events.on("MenuCapabilities", ({ version, usage_reader }) => { this.menuVersion = version; this.usageReader = !!usage_reader && (!this.transport?.remote || !!this.transport.usage); if (this.usageReader) this.send({ SetUsageReader: { enabled: true } }) })
    this.events.on("MenuResult", (result) => {
      const matched = this.operations.complete(result)
      if (matched && result.exit && !result.error) {
        this.detach()
        this.onControl({ kind: "exit" })
      }
    })
  }

  async start(sessionId: string): Promise<void> {
    await this.attach(sessionId, false)
  }

  send(request: unknown): void {
    if (!this.socket || this.closed || this.detached) return
    this.socket.write(encodeFrame(request))
  }

  async sendMenu(mode: string, action: import("./modes/actions.ts").MenuAction): Promise<string | null> {
    if (!this.socket || this.closed || this.detached) return "Not connected"
    if (this.menuVersion < 1) return "This daemon needs menu protocol v1. Restart the session daemon with the current Koma build."
    const request_id = crypto.randomUUID()
    const pending = this.operations.wait(request_id)
    this.send({ Menu: { request_id, mode, action } })
    return pending
  }

  /** Correlated composer flush; unrelated key Acks cannot submit stale text. */
  async sendTracked(request: unknown): Promise<string> {
    if (!this.socket || this.closed || this.detached) return "not connected"
    if (this.menuVersion < 1) return "This daemon needs the current menu protocol. Restart it to enable editable inputs."
    if (!request || typeof request !== "object" || !("ReplaceInput" in request)) return "unsupported tracked operation"
    const request_id = crypto.randomUUID()
    const pending = this.operations.wait(request_id)
    this.send({ ReplaceInputTracked: { request_id, ...(request as { ReplaceInput: { text: string; cursor: number } }).ReplaceInput } })
    return (await pending) ?? "ack"
  }

  private async attach(sessionId: string, restarted: boolean): Promise<void> {
    if (!this.transport?.remote) await ensureDaemon(sessionId)
    if (this.closed) return
    const hello = this.expectHello()
    await this.connect(sessionId)
    const version = await hello
    if (this.transport?.remote) return
    if (!needsRestart(version, process.env.KOMA_BUILD, restarted) || this.closed) return
    this.restarting = true
    this.dropSocket()
    this.snap = null
    this.onUpdate(null)
    await killSession(sessionId)
    this.restarting = false
    if (this.closed) return
    await this.attach(sessionId, true)
  }

  onDisconnect: (() => void) | null = null
  usageReader = false

  private expectHello(): Promise<string | null> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        if (this.helloResolve) {
          this.helloResolve = null
          resolve(null)
        }
      }, 2000)
      this.helloResolve = (version) => {
        clearTimeout(timer)
        this.helloResolve = null
        resolve(version)
      }
    })
  }

  private async connect(sessionId: string): Promise<void> {
    this.usageReader = false
    this.reader = new FrameReader()
    this.seq = -1
    this.resyncing = false
    if (this.transport) {
      await this.transport.open(sessionId, (data) => this.onData(data), () => this.onClose(), (error) => this.fail(error))
      if (this.closed || this.detached) { this.transport.end(); return }
      this.socket = this.transport
      this.send({ Attach: { foreground_id: null, cwd: null } })
      this.send("GetMenuCapabilities")
      return
    }
    const socket = await new Promise<Socket>((resolve, reject) => {
      const connection = createConnection({ path: socketPath(sessionId) })
      let connected = false
      connection.once("connect", () => { connected = true; resolve(connection) })
      connection.on("data", (data: Buffer) => this.onData(data))
      connection.on("error", (error) => { if (connected) this.fail(error); else reject(error) })
      connection.once("close", () => { if (connected) this.onClose() })
    })
    if (this.closed) {
      socket.end()
      return
    }
    this.socket = socket
    this.send({
      Attach: { foreground_id: null, cwd: process.cwd() },
    })
    this.send("GetMenuCapabilities")
  }

  private dropSocket(): void {
    this.helloResolve?.(null)
    const socket = this.socket
    this.socket = null
    try {
      socket?.end()
    } catch {
      // The restart is about to kill the process anyway.
    }
  }

  detach(): void {
    if (this.detached) return
    this.helloResolve?.(null)
    this.operations.close()
    this.detached = true
    this.closed = true
    try {
      this.sendRaw("Detach")
    } catch {
      // The daemon drops us on a closed socket either way.
    }
    try {
      this.socket?.end()
    } catch {
      // Already gone.
    }
    this.socket = null
  }

  private sendRaw(request: unknown): void {
    this.socket?.write(encodeFrame(request))
  }

  private onData(data: Uint8Array | Buffer): void {
    let frames: unknown[]
    try {
      frames = this.reader.push(data instanceof Uint8Array ? data : new Uint8Array(data))
    } catch (error) {
      this.fail(error)
      return
    }
    countFrames(data.length, frames)
    for (const frame of frames) this.onFrame(frame)
  }

  private onFrame(frame: unknown): void {
    if (this.closed || this.detached) return
    if (!frame || typeof frame !== "object") return
    const envelope = frame as { seq?: number; event?: unknown }
    if (typeof envelope.seq === "number") {
      if (this.seq >= 0 && envelope.seq !== this.seq + 1) this.send("Resync")
      this.seq = envelope.seq
    }
    const event = envelope.event
    this.events.dispatch(event)
    if (this.detached || this.closed) return
    if (event && typeof event === "object" && "Hello" in (event as object)) {
      const hello = (event as { Hello?: { version?: string; capabilities?: unknown } }).Hello ?? {}
      if (Array.isArray(hello.capabilities) && hello.capabilities.includes("session_progress")) {
        this.send({ SetProgressDeltas: { enabled: true } })
      }
      this.helloResolve?.(String(hello.version ?? ""))
      return
    }
    if (event === "Ack") return
    if (isSnapshotEvent(event)) this.resyncing = false
    else if (event && typeof event === "object" && "Delta" in event) {
      if (this.resyncing) return
      if (needsResync(this.snap, event)) {
        this.resyncing = true
        this.send("Resync")
        return
      }
    }
    const control = controlOf(event)
    if (control.kind !== "frame") {
      this.onControl(control)
      if (control.kind === "home" || control.kind === "new" || control.kind === "open") return
    }
    const next = fold(this.snap, event)
    if (next !== this.snap) {
      this.snap = next
      this.onUpdate(next)
    }
  }

  private onClose(): void {
    if (this.detached || this.restarting || this.closed) return
    this.fail(new Error(this.menuVersion ? "daemon closed the connection" : "The daemon does not support terminal menu protocol v1. Update the host daemon, then reconnect."))
  }

  private fail(error: unknown): void {
    if (this.detached || this.restarting || this.closed) return
    const text = error instanceof Error ? error.message : String(error)
    this.closed = true
    this.operations.close(text)
    this.onDisconnect?.()
    this.dropSocket()
    this.onControl({ kind: "disconnect", text })
  }
}
