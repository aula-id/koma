import { createConnection } from "node:net"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { existsSync, readFileSync, realpathSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const NAMESPACE_OID = "6ba7b812-9dad-11d1-80b4-00c04fd430c8"

export interface LiveSession {
  session_id: string
  name: string
  pwd: string
  working: boolean
  is_foreground?: boolean
}

export interface HistorySession {
  session_id: string
  name: string
  pwd: string
  updated_at: number
  dir_label: string
}

export interface SessionsFile {
  live: LiveSession[]
  history: HistorySession[]
}

/** The launcher passes Rust's resolved root, including Windows folder redirection. */
export function dataDir(): string {
  const supplied = process.env.KOMA_DATA_DIR || process.env.KOMA_TEST_DATA_DIR
  if (supplied) return supplied
  if (process.platform === "win32") {
    const local = process.env.LOCALAPPDATA
    if (!local) throw new Error("cannot resolve %LOCALAPPDATA%")
    return join(local, "koma")
  }
  return join(homedir(), ".koma")
}

/** Palette saved in the data directory. The hub stays dark; a session uses this before its snapshot arrives. */
export function savedPalette(): string {
  const dir = dataDir()
  try {
    const raw = JSON.parse(readFileSync(join(dir, "config.json"), "utf8")) as { palette?: unknown }
    return typeof raw.palette === "string" && raw.palette.trim() ? raw.palette : "dark"
  } catch {
    return "dark"
  }
}

export function launchPalette(kind: "hub" | "session" | "remote", saved: string): string {
  return kind === "hub" ? "dark" : saved || "dark"
}

export function komaBin(): string {
  const fromEnv = process.env.KOMA_BIN
  if (fromEnv && existsSync(fromEnv)) return fromEnv
  const here = fileURLToPath(new URL(".", import.meta.url))
  const debug = join(here, "../../target/debug/koma")
  if (existsSync(debug)) return debug
  return fromEnv && fromEnv.length > 0 ? fromEnv : "koma"
}

export function socketPath(sessionId: string, platform = process.platform): string {
  if (platform === "win32") return String.raw`\\.\pipe\koma-${sessionId}`
  return join(dataDir(), "run", `${sessionId}.sock`)
}

/** UUID v5, simple hex, matching Rust `Uuid::new_v5(...).simple()`. */
export function uuidV5Simple(namespaceUuid: string, name: string): string {
  const namespace = Buffer.from(namespaceUuid.replace(/-/g, ""), "hex")
  const digest = createHash("sha1").update(namespace).update(name, "utf8").digest()
  digest[6] = ((digest[6] ?? 0) & 0x0f) | 0x50
  digest[8] = ((digest[8] ?? 0) & 0x3f) | 0x80
  return digest.subarray(0, 16).toString("hex")
}

/** The same data root and working-directory bucket used by the daemon. */
export function sessionDir(cwd: string, sessionId: string): string {
  let path = cwd
  try {
    path = realpathSync(cwd)
  } catch {
    // Missing dirs hash as given, same as the daemon.
  }
  return join(dataDir(), "sessions", uuidV5Simple(NAMESPACE_OID, path), sessionId)
}

export function attachmentFile(cwd: string, sessionId: string, relPath: string): string {
  return join(sessionDir(cwd, sessionId), relPath)
}

export async function listSessions(): Promise<SessionsFile> {
  const proc = Bun.spawn([komaBin(), "sessions", "--json"], {
    stdout: "pipe",
    stderr: "pipe",
    stdin: "ignore",
  })
  const text = await new Response(proc.stdout).text()
  const code = await proc.exited
  if (code !== 0) {
    const err = await new Response(proc.stderr).text()
    throw new Error(err.trim() || `koma sessions --json exited ${code}`)
  }
  const trimmed = text.trim()
  if (!trimmed) return { live: [], history: [] }
  const parsed = JSON.parse(trimmed) as unknown
  if (Array.isArray(parsed)) return { live: parsed as LiveSession[], history: [] }
  const body = parsed as Partial<SessionsFile>
  return {
    live: Array.isArray(body.live) ? body.live : [],
    history: Array.isArray(body.history) ? body.history : [],
  }
}

export async function killSession(sessionId: string): Promise<void> {
  const proc = Bun.spawn([komaBin(), "daemon", "kill", "--session", sessionId], {
    stdout: "ignore",
    stderr: "pipe",
    stdin: "ignore",
  })
  await proc.exited
}

export async function deleteSession(sessionId: string): Promise<void> {
  const proc = Bun.spawn([komaBin(), "daemon", "delete", "--session", sessionId], {
    stdout: "ignore",
    stderr: "pipe",
    stdin: "ignore",
  })
  await proc.exited
}

/** Sessions whose socket must not be filled by a coding daemon. */
const daemonHolds = new Set<string>()

export function holdDaemon(sessionId: string): void {
  daemonHolds.add(sessionId)
}

export function releaseDaemon(sessionId: string): void {
  daemonHolds.delete(sessionId)
}

export function pidPath(sessionId: string): string {
  return join(dataDir(), "run", `${sessionId}.pid`)
}

export function sessionLockPath(cwd: string, sessionId: string): string {
  return join(sessionDir(cwd, sessionId), "session.lock")
}

/** Start a session daemon. `secure` runs the room instead of the coding daemon. */
export function spawnDaemon(sessionId: string, kind?: "secure"): void {
  const args = ["--daemon", "--session", sessionId]
  if (kind === "secure") args.push("--kind", "secure")
  const child = spawn(komaBin(), args, {
    detached: true,
    stdio: "ignore",
    cwd: process.cwd(),
  })
  child.unref()
}

export async function ensureDaemon(sessionId: string, waitMs = 8000): Promise<void> {
  const path = socketPath(sessionId)
  if (await probeSocket(path)) return
  if (daemonHolds.has(sessionId)) {
    const deadline = Date.now() + waitMs
    while (Date.now() < deadline) {
      if (await probeSocket(path)) return
      await Bun.sleep(50)
    }
    throw new Error(`session ${sessionId} is switching daemons`)
  }
  // The daemon unlinks a stale socket itself before it binds.
  spawnDaemon(sessionId)
  const deadline = Date.now() + waitMs
  while (Date.now() < deadline) {
    if (await probeSocket(path)) return
    await Bun.sleep(100)
  }
  throw new Error(`session daemon did not accept on ${path}`)
}

/** True when the socket, the pid file, and the session lock are all gone. */
export function daemonReleased(sessionId: string, cwd = process.cwd()): boolean {
  if (process.platform !== "win32" && existsSync(socketPath(sessionId))) return false
  if (existsSync(pidPath(sessionId))) return false
  if (existsSync(sessionLockPath(cwd, sessionId))) return false
  return true
}

/** Stop one daemon and start the other. The hold blocks `ensureDaemon` while the socket is empty. */
export async function switchDaemon(sessionId: string, kind: "secure" | "coding", cwd = process.cwd()): Promise<void> {
  holdDaemon(sessionId)
  try {
    await killSession(sessionId)
    const deadline = Date.now() + 8000
    while (!daemonReleased(sessionId, cwd)) {
      if (Date.now() > deadline) throw new Error(`session ${sessionId} did not release its daemon`)
      if (process.platform === "win32" && await probeSocket(socketPath(sessionId))) {
        await Bun.sleep(50)
        continue
      }
      await Bun.sleep(50)
    }
    spawnDaemon(sessionId, kind === "secure" ? "secure" : undefined)
    const up = Date.now() + 8000
    while (!(await probeSocket(socketPath(sessionId)))) {
      if (Date.now() > up) throw new Error(`session daemon did not accept on ${socketPath(sessionId)}`)
      await Bun.sleep(100)
    }
  } finally {
    releaseDaemon(sessionId)
  }
}

async function probeSocket(path: string): Promise<boolean> {
  if (process.platform !== "win32" && !existsSync(path)) return false
  return new Promise((resolve) => {
    const socket = createConnection({ path })
    const finish = (connected: boolean) => { socket.destroy(); resolve(connected) }
    socket.once("connect", () => finish(true))
    socket.once("error", () => finish(false))
    socket.setTimeout(500, () => finish(false))
  })
}
