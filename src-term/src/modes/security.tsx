import type { ReactNode } from "react"
import { useEffect, useState } from "react"
import type { MenuData } from "./data.ts"
import type { Theme } from "../theme.ts"
import { Action, Lines, Panel, Toggle, sendKey, useBodyHeight, type Line } from "./bits.tsx"
import { flag, num, rec, rows, str, type Rec } from "./read.ts"

type Send = (request: unknown) => void
type Role = "success" | "warn" | "error" | "info" | "dim"

const MAP_DOMAINS = ["recon", "web", "identity", "cloud", "rev", "crypto", "pxn", "dfir", "lab", "minimal"]
const MAP_KINDS = ["", "-2", "-ext", "-src"]
/** Mirrors `PSEUDO_IMAGES` in the daemon: these never take pull/activate/remove. */
const PSEUDO_IMAGES = ["", "smoke", "none", "external", "debian", "empty"]

/** Same filter the daemon applies, so the cursor indexes the same window. */
export function catalogRows(body: Rec): Rec[] {
  const filter = str(body.catalog_filter) || "all"
  const query = str(body.catalog_query).toLowerCase()
  const map = str(body.map_filter)
  return rows(body.catalog).filter((row) => {
    const mark = str(row.ready_mark) || "remote"
    if (filter === "ready" && mark !== "ready") return false
    if (filter === "needs" && mark === "ready") return false
    if (map && str(row.image) !== map) return false
    if (query && !str(row.id).toLowerCase().includes(query) && !str(row.blurb).toLowerCase().includes(query)) return false
    return true
  })
}

const READINESS: Record<string, { glyph: string; text: string; reason: string; role: Role }> = {
  ready: { glyph: "●", text: "ready", reason: "in the active guest", role: "success" },
  missing: { glyph: "×", text: "missing", reason: "image active, binary not in it", role: "warn" },
  local: { glyph: "▒", text: "local", reason: "pulled, not active", role: "info" },
  remote: { glyph: "·", text: "remote", reason: "not pulled", role: "dim" },
}
const readiness = (mark: string) => READINESS[mark] ?? READINESS.remote!

// ─── Image pull activity ──────────────────────────────────────────────────────

const SPIN = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

/** Local animation clock: advances only while `active`, cleared on unmount. */
function useTick(active: boolean, ms = 90): number {
  const [tick, setTick] = useState(0)
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => setTick((value) => (value + 1) % 1_000_000), ms)
    return () => clearInterval(timer)
  }, [active, ms])
  return tick
}

type Pull = { id: string; phase: string; current: number; total: number; bps: number; layersDone: number; layers: number; elapsed: number }

function pullOf(body: Rec): Pull | null {
  const raw = body.image_pull
  if (!raw || typeof raw !== "object") return null
  const p = raw as Rec
  if (!str(p.id)) return null
  return { id: str(p.id), phase: str(p.phase) || "starting", current: num(p.current), total: num(p.total), bps: num(p.bps), layersDone: num(p.layers_done), layers: num(p.layers), elapsed: num(p.elapsed_ms) }
}

function pullResultOf(body: Rec): { id: string; ok: boolean; message: string } | null {
  const raw = body.image_pull_result
  if (!raw || typeof raw !== "object") return null
  const r = raw as Rec
  return { id: str(r.id), ok: flag(r.ok), message: str(r.message) }
}

const pullRatio = (pull: Pull) => pull.total > 0 ? Math.max(0, Math.min(1, pull.current / pull.total)) : null

function bytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`
  if (n >= 1e3) return `${(n / 1e3).toFixed(0)} kB`
  return `${n} B`
}

function clock(ms: number): string {
  const total = Math.floor(ms / 1000)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const sec = String(total % 60).padStart(2, "0")
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`
}

function phaseText(pull: Pull): string {
  switch (pull.phase) {
    case "starting": return "starting"
    case "contacting": return "contacting registry"
    case "waiting": return pull.layers ? `waiting for layers ${pull.layersDone}/${pull.layers}` : "waiting for layers"
    case "downloading": return "downloading"
    case "extracting": return pull.total ? "extracting" : `extracting ${pull.layersDone}/${Math.max(1, pull.layers)} layers`
    case "verifying": return "verifying layers"
    case "preparing": return "preparing image"
    case "finishing": return "finishing"
    default: return pull.phase
  }
}

const EIGHTHS = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"]

/** Determinate bar with eighth-cell resolution, or a sweeping block when the size is unknown. */
function PullBar({ pull, tick, cells, theme }: { pull: Pull; tick: number; cells: number; theme: Theme }) {
  const ratio = pullRatio(pull)
  if (ratio == null) {
    const width = Math.max(2, Math.floor(cells / 4))
    const span = cells + width
    const head = tick % span
    const chars = Array.from({ length: cells }, (_, i) => (i <= head && i > head - width ? "█" : "░")).join("")
    return <span fg={theme.info}>{chars}</span>
  }
  const exact = ratio * cells
  const full = Math.floor(exact)
  const part = EIGHTHS[Math.floor((exact - full) * 8)] ?? ""
  const rest = Math.max(0, cells - full - (part ? 1 : 0))
  return <span><span fg={theme.success}>{"█".repeat(full)}{part}</span><span fg={theme.dim}>{"░".repeat(rest)}</span></span>
}

/** One-line live pull readout: spinner, id, bar, percent, bytes, rate, clock. */
function PullLine({ pull, theme, width }: { pull: Pull; theme: Theme; width: number }) {
  const tick = useTick(true)
  const ratio = pullRatio(pull)
  const cells = width >= 100 ? 24 : width >= 70 ? 16 : 10
  const pct = ratio == null ? "" : ` ${(ratio * 100).toFixed(ratio < 0.1 ? 1 : 0)}%`
  const amount = pull.total ? `${bytes(pull.current)} / ${bytes(pull.total)}` : pull.current ? bytes(pull.current) : ""
  const rate = pull.bps ? `${bytes(pull.bps)}/s` : ""
  const tail = [width >= 70 ? phaseText(pull) : "", amount, width >= 60 ? rate : "", clock(pull.elapsed)].filter(Boolean).join("  ")
  return (
    <text wrapMode="none">
      <span fg={theme.accent}>{`${SPIN[tick % SPIN.length]} `}</span>
      <span fg={theme.dim}>pulling </span><span fg={theme.fg}>{pull.id}</span><span>{"  "}</span>
      <PullBar pull={pull} tick={tick} cells={cells} theme={theme} />
      <span fg={theme.fg}>{pct}</span>
      <span fg={theme.dim}>{`  ${tail}`}</span>
    </text>
  )
}

/** Short badge for a table row whose image is pulling. */
function pullBadge(pull: Pull, tick: number): string {
  const ratio = pullRatio(pull)
  return `${SPIN[tick % SPIN.length]} ${ratio == null ? "pull" : `${Math.floor(ratio * 100)}%`}`.padEnd(8)
}

function posture(body: Rec): { text: string; role: Role } {
  const status = rec(body.status)
  if (str(body.agent_mode_label) === "yolo") return { text: "■ YOLO ACTIVE", role: "error" }
  if (flag(body.yolo_armed)) return { text: "▲ ARMED", role: "warn" }
  if (flag(status.running)) return { text: "● ONLINE", role: "success" }
  return { text: "○ OFFLINE", role: "dim" }
}

export function securityView(body: MenuData["Security"] & Rec, theme: Theme, onMove: (index: number) => void, send: Send) {
  return {
    title: "security",
    hint: "security",
    children: <SecurityConsole body={body} theme={theme} onMove={onMove} send={send} />,
  }
}

function SecurityConsole({ body, theme, onMove, send }: { body: Rec; theme: Theme; onMove: (index: number) => void; send: Send }) {
  const { ref, width, height } = useBodyHeight(20)
  const pane = str(body.pane) || "tools"
  return (
    <box ref={ref} flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0} gap={1}>
      <Header body={body} pane={pane} theme={theme} send={send} compact={width < 64} />
      {pane === "images" ? <ImagesPane body={body} theme={theme} onMove={onMove} send={send} width={width} />
        : pane === "refused" ? <RefusedPane body={body} theme={theme} onMove={onMove} send={send} />
          : <Dashboard body={body} theme={theme} onMove={onMove} send={send} width={width} height={height} />}
    </box>
  )
}

function Header({ body, pane, theme, send, compact }: { body: Rec; pane: string; theme: Theme; send: Send; compact: boolean }) {
  const tabs = [["tools", "Dashboard (1)"], ["images", "Images (2)"], ["refused", "Refused (3)"]] as const
  const state = posture(body)
  const refused = rows(rec(body.status).refused).length
  return (
    <box flexDirection="row" flexShrink={0} height={1} minWidth={0}>
      {tabs.map(([id, label], index) => (
        <box key={id} flexDirection="row" flexShrink={0}>
          {index > 0 ? <text fg={theme.dim}>|</text> : null}
          <Action label={id === "refused" && refused ? `Refused ${refused} (3)` : label} primary={pane === id} theme={theme} onPress={() => { if (pane !== id) send({ MenuReplace: { field: "pane", value: id } }) }} />
        </box>
      ))}
      <box flexGrow={1} minWidth={1} />
      <text flexShrink={0} wrapMode="none" fg={theme[state.role]}>{compact ? ` ${state.text.slice(0, 1)} ` : ` ${state.text} `}</text>
    </box>
  )
}

// ─── Dashboard ────────────────────────────────────────────────────────────────

function Dashboard({ body, theme, onMove, send, width, height }: { body: Rec; theme: Theme; onMove: (index: number) => void; send: Send; width: number; height: number }) {
  const tiles = width >= 96 && height >= 28
  const map = width >= 110
  // Short terminals fold both control panels into one bar so the tool list keeps rows.
  const short = height < 18
  return (
    <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0} gap={tiles ? 1 : 0}>
      {tiles ? <StatusBoard body={body} theme={theme} /> : pullOf(body) ? <box height={1} flexShrink={0} minWidth={0} overflow="hidden"><PullLine pull={pullOf(body)!} theme={theme} width={width} /></box> : <StatusStrip body={body} theme={theme} />}
      {short ? <ControlBar body={body} theme={theme} send={send} /> : (
        <box flexDirection="row" flexShrink={0} columnGap={1} minWidth={0}>
          <box flexGrow={1} flexBasis={0} minWidth={0}><DockerControl body={body} theme={theme} send={send} /></box>
          <box flexGrow={1} flexBasis={0} minWidth={0}><Operations body={body} theme={theme} send={send} /></box>
        </box>
      )}
      <box flexDirection="row" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0} columnGap={1}>
        <ToolsPanel body={body} theme={theme} onMove={onMove} send={send} width={map ? width - 31 : width} showMap={!map} />
        {map ? <GuestMap body={body} theme={theme} /> : null}
      </box>
    </box>
  )
}

/** Where the ready count came from: the probed guest, or an estimate from the pin. */
export function catalogDetail(status: Rec): string {
  const source = str(status.catalog_source)
  if (source === "guest") return "in guest"
  if (source === "checking") return "checking…"
  if (source === "index") return "estimate"
  return ""
}

type Gauge = { title: string; up: boolean | null; value: string; detail: string; bar?: number; pull?: Pull }

function gauges(body: Rec): Gauge[] {
  const status = rec(body.status)
  const images = rows(body.images)
  const active = images.find((row) => flag(row.active))
  const catalog = rows(body.catalog)
  const total = num(status.catalog_total) || catalog.length
  const ready = num(status.catalog_ready) || catalog.filter((row) => str(row.ready_mark) === "ready").length
  const net = str(status.net) || "unknown"
  const rules = Array.isArray(status.rules) ? status.rules.length : 0
  const running = flag(status.running)
  return [
    { title: "Runtime", up: flag(status.runtime_ok), value: flag(status.runtime_ok) ? "up" : "down", detail: str(status.runtime) || "no container runtime" },
    { title: "Daemon", up: running, value: running ? "running" : "stopped", detail: flag(status.installed) ? `${rows(status.tools).length} tools advertised` : "not installed" },
    { title: "Guest", up: !!active, value: active ? str(active.id) : "none", detail: `${images.filter((row) => flag(row.local)).length}/${images.length} images pulled`, pull: pullOf(body) ?? undefined },
    { title: "Boundary", up: flag(status.boundary_up), value: flag(status.boundary_up) ? "up" : "down", detail: flag(status.boundary_up) ? "guest isolated" : "no boundary guest" },
    { title: "Egress", up: net === "off" || net === "scope" ? true : net === "down" ? false : null, value: net, detail: `${rules} allow rule${rules === 1 ? "" : "s"}` },
    { title: "Catalog", up: ready > 0, value: `${ready}/${total}`, detail: catalogDetail(status), bar: total ? ready / total : 0 },
  ]
}

const gaugeGlyph = (up: boolean | null) => up == null ? "◌" : up ? "●" : "○"
const gaugeColor = (theme: Theme, up: boolean | null) => up == null ? theme.dim : up ? theme.success : theme.warn

function StatusBoard({ body, theme }: { body: Rec; theme: Theme }) {
  return (
    <box flexDirection="row" flexShrink={0} height={4} columnGap={1} minWidth={0}>
      {gauges(body).map((gauge) => (
        <box key={gauge.title} border borderStyle="rounded" borderColor={theme.dim} title={` ${gauge.title} `} titleColor={theme.dim} flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden" flexDirection="column" paddingLeft={1} paddingRight={1}>
          <text wrapMode="none"><span fg={gaugeColor(theme, gauge.up)}>{`${gaugeGlyph(gauge.up)} `}</span><span fg={theme.fg}>{gauge.value}</span>{gauge.bar != null && gauge.detail ? <span fg={theme.dim}>{` · ${gauge.detail}`}</span> : null}</text>
          {gauge.pull ? <TilePull pull={gauge.pull} theme={theme} /> : gauge.bar != null ? <Bar ratio={gauge.bar} theme={theme} /> : <text wrapMode="none" fg={theme.dim}>{gauge.detail}</text>}
        </box>
      ))}
    </box>
  )
}

function Bar({ ratio, theme }: { ratio: number; theme: Theme }) {
  const cells = 10
  const filled = Math.round(Math.max(0, Math.min(1, ratio)) * cells)
  return <text wrapMode="none"><span fg={theme.success}>{"█".repeat(filled)}</span><span fg={theme.dim}>{"░".repeat(cells - filled)}</span><span fg={theme.dim}>{` ${Math.round(ratio * 100)}%`}</span></text>
}

/** Guest tile while pulling: spinner, target, compact bar. */
function TilePull({ pull, theme }: { pull: Pull; theme: Theme }) {
  const tick = useTick(true)
  const ratio = pullRatio(pull)
  return (
    <text wrapMode="none">
      <span fg={theme.accent}>{`${SPIN[tick % SPIN.length]} `}</span>
      <PullBar pull={pull} tick={tick} cells={8} theme={theme} />
      <span fg={theme.fg}>{ratio == null ? ` ${pull.id}` : ` ${Math.floor(ratio * 100)}% ${pull.id}`}</span>
    </text>
  )
}

function StatusStrip({ body, theme }: { body: Rec; theme: Theme }) {
  return (
    <box flexShrink={0} height={1} minWidth={0} overflow="hidden">
      <text wrapMode="none">
        {gauges(body).map((gauge, index) => (
          <span key={gauge.title}>
            {index ? <span fg={theme.dim}>{"  "}</span> : null}
            <span fg={gaugeColor(theme, gauge.up)}>{gaugeGlyph(gauge.up)}</span>
            <span fg={theme.dim}>{` ${gauge.title.toLowerCase()} `}</span>
            <span fg={theme.fg}>{gauge.value}</span>
          </span>
        ))}
      </text>
    </box>
  )
}

/** Type B row: label left, value, control reserved on the right edge. */
function ControlRow({ label, value, color, theme, control, onPress }: { label: string; value: string; color?: string; theme: Theme; control: ReactNode; onPress?: () => void }) {
  const [hot, setHot] = useState(false)
  return (
    <box height={1} flexDirection="row" flexShrink={0} minWidth={0} paddingLeft={1} paddingRight={1} backgroundColor={onPress && hot ? theme.panel : undefined} onMouseOver={() => setHot(true)} onMouseOut={() => setHot(false)} onMouseDown={onPress}>
      <text width={8} flexShrink={0} fg={theme.dim} wrapMode="none">{label}</text>
      <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden"><text wrapMode="none" fg={color ?? theme.fg}>{value}</text></box>
      <box flexShrink={0} flexDirection="row" paddingLeft={1}>{control}</box>
    </box>
  )
}

function DockerControl({ body, theme, send }: { body: Rec; theme: Theme; send: Send }) {
  const status = rec(body.status)
  const running = flag(status.running)
  const runtimeOk = flag(status.runtime_ok)
  return (
    <Panel title="Docker control" maxLines={3} hot={false} theme={theme}>
      <ControlRow label="Daemon" value={running ? "running" : flag(status.installed) ? "stopped" : "not installed"} color={running ? theme.success : theme.dim} theme={theme} onPress={() => sendKey(send, { Char: "s" })} control={<><Toggle checked={running} theme={theme} /><text fg={theme.dim}> (s) </text></>} />
      <ControlRow label="Runtime" value={str(status.runtime) || (runtimeOk ? "ready" : "unavailable")} color={runtimeOk ? theme.fg : theme.warn} theme={theme} control={<Action label="Ensure (c)" plain theme={theme} onPress={() => sendKey(send, { Char: "c" })} />} />
      <ControlRow label="Index" value="catalog + images" color={theme.dim} theme={theme} control={<Action label="Refresh (u)" plain theme={theme} onPress={() => sendKey(send, { Char: "u" })} />} />
    </Panel>
  )
}

function Operations({ body, theme, send }: { body: Rec; theme: Theme; send: Send }) {
  const running = flag(rec(body.status).running)
  const armed = flag(body.yolo_armed)
  const mode = str(body.agent_mode_label) || "auto"
  const yolo = mode === "yolo"
  const locked = !(running && armed)
  return (
    <Panel title="Operations" maxLines={3} hot={false} theme={theme} status={clipNotice(str(body.notice), 44)}>
      <ControlRow label="YOLO" value={armed ? "armed" : "locked"} color={armed ? theme.warn : theme.dim} theme={theme} onPress={() => sendKey(send, { Char: "o" })} control={<><Toggle checked={armed} theme={theme} /><text fg={theme.dim}> (o) </text></>} />
      <ControlRow label="Mode" value={locked && !yolo ? `${mode} · needs daemon + arm` : mode} color={yolo ? theme.error : theme.fg} theme={theme} control={<Action label={yolo ? "Leave (e)" : "Enter (e)"} plain disabled={locked} theme={theme} onPress={() => sendKey(send, { Char: "e" })} />} />
      <LaneRow body={body} theme={theme} send={send} />
    </Panel>
  )
}

const LANES = [["none", "None"], ["research", "Research"], ["red", "Red"], ["blue", "Blue"]] as const

/** Direct lane pick; the active lane uses the primary style. `l` still cycles. */
function LaneChoices({ body, theme, send }: { body: Rec; theme: Theme; send: Send }) {
  const current = (str(body.lane_label) || "none").toLowerCase()
  return (
    <box flexDirection="row" flexShrink={0}>
      {LANES.map(([id, label], index) => (
        <box key={id} flexDirection="row" flexShrink={0}>
          {index ? <text fg={theme.dim}>·</text> : null}
          <Action label={index === LANES.length - 1 ? `${label} (l)` : label} plain primary={current === id} theme={theme} onPress={() => { if (current !== id) send({ MenuReplace: { field: "lane", value: id } }) }} />
        </box>
      ))}
    </box>
  )
}

function LaneRow({ body, theme, send }: { body: Rec; theme: Theme; send: Send }) {
  return (
    <box height={1} flexDirection="row" flexShrink={0} minWidth={0} paddingLeft={1} paddingRight={1}>
      <text width={8} flexShrink={0} fg={theme.dim} wrapMode="none">Lane</text>
      <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden" flexDirection="row">
        <LaneChoices body={body} theme={theme} send={send} />
      </box>
    </box>
  )
}

/** Same controls as Docker control + Operations, on one wrapping row. */
function ControlBar({ body, theme, send }: { body: Rec; theme: Theme; send: Send }) {
  const running = flag(rec(body.status).running)
  const armed = flag(body.yolo_armed)
  const mode = str(body.agent_mode_label) || "auto"
  const key = (c: string) => () => sendKey(send, { Char: c })
  const toggle = (label: string, on: boolean, k: string) => (
    <box key={k} flexDirection="row" flexShrink={0} paddingLeft={1} paddingRight={1} onMouseDown={key(k)}>
      <text fg={theme.fg}>{`${label} `}</text><Toggle checked={on} theme={theme} /><text fg={theme.dim}>{` (${k})`}</text>
    </box>
  )
  return (
    <box flexDirection="row" flexWrap="wrap" flexShrink={0} minWidth={0}>
      {toggle("Daemon", running, "s")}
      <Action label="Ensure runtime (c)" plain theme={theme} onPress={key("c")} />
      <Action label="Refresh (u)" plain theme={theme} onPress={key("u")} />
      <text fg={theme.dim}>|</text>
      {toggle("YOLO", armed, "o")}
      <Action label={mode === "yolo" ? "Leave (e)" : "Enter (e)"} plain disabled={!(running && armed)} theme={theme} onPress={key("e")} />
      <text fg={theme.dim}>{" Lane "}</text>
      <LaneChoices body={body} theme={theme} send={send} />
      {str(body.notice) ? <text fg={noticeColor(theme, str(body.notice))} wrapMode="none">{` ${clipNotice(str(body.notice), 60)}`}</text> : null}
    </box>
  )
}

function clipNotice(text: string, max: number): string | undefined {
  if (!text) return undefined
  const chars = [...text]
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text
}

const noticeColor = (theme: Theme, text: string) => /fail|refused|not set|error|cannot|could not/i.test(text) ? theme.warn : theme.dim

function toolOps(body: Rec, tool: Rec | undefined): { label: string; key: string }[] {
  if (!tool) return []
  const image = str(tool.image)
  if (PSEUDO_IMAGES.includes(image.trim().toLowerCase())) return []
  const mark = str(tool.ready_mark)
  if (mark === "remote") return [{ label: "Pull (p)", key: "p" }]
  if (mark !== "local") return []
  const row = rows(body.images).find((img) => str(img.id).toLowerCase() === image.toLowerCase())
  const ops = [{ label: "Activate (a)", key: "a" }]
  if (row && flag(row.local) && !flag(row.active)) ops.push({ label: "Remove (x)", key: "x" })
  return ops
}

function ToolsPanel({ body, theme, onMove, send, width, showMap }: { body: Rec; theme: Theme; onMove: (index: number) => void; send: Send; width: number; showMap: boolean }) {
  const pull = pullOf(body)
  const tick = useTick(!!pull)
  const catalog = catalogRows(body)
  const selected = Math.min(num(body.selected), Math.max(0, catalog.length - 1))
  const tool = catalog[selected]
  const wideCols = width >= 80
  const idWidth = wideCols ? 18 : 14
  const imageWidth = wideCols ? 11 : 9
  const filter = str(body.catalog_filter) || "all"
  const searching = flag(body.catalog_searching)
  const query = str(body.catalog_query)
  const mapFilter = str(body.map_filter)
  const lines: Line[] = catalog.map((row) => {
    const state = readiness(str(row.ready_mark))
    const pulling = !!pull && pull.id.toLowerCase() === str(row.image).toLowerCase()
    const cells = [str(row.id).padEnd(idWidth), str(row.image).padEnd(imageWidth), ...(wideCols ? [str(row.exec).padEnd(6)] : []), str(row.blurb)]
    return { label: cells.join(" "), split: true, badge: pulling ? { text: pullBadge(pull!, tick), role: "info" } : { text: `${state.glyph} ${state.text}`.padEnd(8), role: state.role } }
  })
  const header = ["TOOL".padEnd(idWidth), "IMAGE".padEnd(imageWidth), ...(wideCols ? ["EXEC  "] : []), "DESCRIPTION"].join(" ")
  const actions = (
    <>
      <Action label={`Filter ${filter} (f)`} primary={filter !== "all"} theme={theme} onPress={() => sendKey(send, { Char: "f" })} />
      {searching || query
        ? <text fg={theme.accent} wrapMode="none">{` /${query}${searching ? "▮" : ""} `}</text>
        : <Action label="Search (/)" theme={theme} onPress={() => sendKey(send, { Char: "/" })} />}
    </>
  )
  const toolPulling = !!pull && !!tool && pull.id.toLowerCase() === str(tool.image).toLowerCase()
  const ops = pull ? toolOps(body, tool).filter((op) => op.key !== "p" && !(toolPulling && op.key === "x")) : toolOps(body, tool)
  const state = readiness(str(tool?.ready_mark))
  return (
    <Panel title={`Tools ${catalog.length}`} hot theme={theme} actions={actions} status={showMap ? `map ${mapFilter || "all"} ←→` : undefined}>
      <box height={1} flexShrink={0} flexDirection="row" paddingLeft={3} paddingRight={1} minWidth={0}>
        <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden"><text fg={theme.dim} wrapMode="none">{header}</text></box>
        <text flexShrink={0} fg={theme.dim} wrapMode="none">{" STATE   "}</text>
      </box>
      {catalog.length
        ? <Lines rows={lines} selected={selected} theme={theme} onPress={onMove} rail />
        : <box flexGrow={1} paddingLeft={1}><text fg={theme.dim}>{rows(body.catalog).length ? "No tools match this filter" : "No catalog rows yet · Refresh (u)"}</text></box>}
      {tool ? (
        <box height={1} flexShrink={0} flexDirection="row" minWidth={0} paddingLeft={1} paddingRight={1} backgroundColor={theme.panel}>
          <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden">
            <text wrapMode="none"><span fg={theme[state.role]}>{`${state.glyph} `}</span><span fg={theme.accent}>{str(tool.id)}</span><span fg={theme.dim}>{`  image ${str(tool.image) || "—"} · exec ${str(tool.exec) || "—"} · ${toolPulling ? `pulling ${pullRatio(pull!) == null ? "" : `${Math.floor(pullRatio(pull!)! * 100)}%`}` : state.reason}`}</span></text>
          </box>
          {ops.map((op) => <Action key={op.key} label={op.label} plain theme={theme} onPress={() => sendKey(send, { Char: op.key })} />)}
        </box>
      ) : null}
    </Panel>
  )
}

function GuestMap({ body, theme }: { body: Rec; theme: Theme }) {
  const images = rows(body.images)
  const byId = new Map(images.map((row) => [str(row.id), row]))
  const filter = str(body.map_filter)
  const cell = (id: string) => {
    const row = byId.get(id)
    if (!row) return <span key={id}>{"    "}</span>
    const glyph = flag(row.active) ? "██" : flag(row.local) ? "▒▒" : "··"
    const color = flag(row.active) ? theme.success : flag(row.local) ? theme.info : theme.dim
    const on = filter === id
    return <span key={id}>{"  "}<span fg={on ? theme.selFg : color} bg={on ? theme.selBg : undefined}>{glyph}</span></span>
  }
  const domains = MAP_DOMAINS.filter((domain) => MAP_KINDS.some((kind) => byId.has(`${domain}${kind}`)))
  return (
    <box width={30} flexShrink={0} flexDirection="column" minHeight={0}>
      <Panel title="Guest map" hot={false} theme={theme} status="←→">
        <box paddingLeft={1} flexDirection="column" flexGrow={1} minHeight={0} overflow="hidden">
          <text wrapMode="none"><span fg={filter ? theme.dim : theme.selFg} bg={filter ? undefined : theme.selBg}>{"all"}</span><span fg={theme.dim}>{"     base  -2 ext src"}</span></text>
          {domains.map((domain) => (
            <text key={domain} wrapMode="none"><span fg={filter.startsWith(domain) ? theme.accent : theme.dim}>{domain.padEnd(8)}</span>{MAP_KINDS.map((kind) => cell(`${domain}${kind}`))}</text>
          ))}
          {!domains.length ? <text fg={theme.dim}>No guest images</text> : null}
          <box flexGrow={1} />
          <text wrapMode="none"><span fg={theme.success}>█</span><span fg={theme.dim}> active  </span><span fg={theme.info}>▒</span><span fg={theme.dim}> local  · remote</span></text>
        </box>
      </Panel>
    </box>
  )
}

// ─── Images & Refused ─────────────────────────────────────────────────────────

function ImagesPane({ body, theme, onMove, send, width }: { body: Rec; theme: Theme; onMove: (index: number) => void; send: Send; width: number }) {
  const pull = pullOf(body)
  const result = pullResultOf(body)
  const tick = useTick(!!pull)
  const images = rows(body.images)
  const selected = num(body.images_selected)
  const active = images.find((row) => flag(row.active))
  const pulled = images.filter((row) => flag(row.local)).length
  const sizeWidth = 8
  const idWidth = Math.min(16, Math.max(10, ...images.map((row) => str(row.id).length + 1)))
  const lines: Line[] = images.map((row) => {
    const pulling = pull?.id === str(row.id)
    const role: Role = pulling ? "info" : flag(row.active) ? "success" : flag(row.local) ? "info" : "dim"
    const text = pulling ? pullBadge(pull!, tick) : flag(row.active) ? "● active" : flag(row.local) ? "▒ local" : "· remote"
    return { label: `${str(row.id).padEnd(idWidth)} ${(str(row.size_label) || "—").padEnd(sizeWidth)} ${str(row.blurb)}`, split: true, badge: { text: text.padEnd(8), role } }
  })
  const key = (c: string) => () => sendKey(send, { Char: c })
  const actions = images[selected] ? (
    <>
      <Action label="Activate (enter)" primary theme={theme} onPress={() => sendKey(send, "Enter")} />
      <Action label="Pull (p)" disabled={!!pull} theme={theme} onPress={key("p")} />
      <Action label="Remove (x)" disabled={pull?.id === str(images[selected]?.id)} theme={theme} onPress={key("x")} />
      {width >= 70 ? <Action label="Refresh (r)" theme={theme} onPress={key("r")} /> : null}
    </>
  ) : <Action label="Refresh (r)" theme={theme} onPress={key("r")} />
  return (
    <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0}>
      <box height={1} flexShrink={0} flexDirection="row" minWidth={0} overflow="hidden">
        {pull ? <PullLine pull={pull} theme={theme} width={width} /> : (
          <text wrapMode="none">
            <span fg={active ? theme.success : theme.dim}>{active ? "● " : "○ "}</span>
            <span fg={theme.dim}>active </span><span fg={theme.fg}>{active ? str(active.id) : "none"}</span>
            <span fg={theme.dim}>{`   pulled ${pulled}/${images.length}`}</span>
            {result ? <span fg={result.ok ? theme.success : theme.error}>{`   ${result.ok ? "✓" : "✗"} ${result.message}`}</span> : null}
          </text>
        )}
      </box>
      <Panel title="Guest images" hot theme={theme} actions={actions}>
        <box height={1} flexShrink={0} flexDirection="row" paddingLeft={3} paddingRight={1} minWidth={0}>
          <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden"><text fg={theme.dim} wrapMode="none">{`${"IMAGE".padEnd(idWidth)} ${"SIZE".padEnd(sizeWidth)} DESCRIPTION`}</text></box>
          <text flexShrink={0} fg={theme.dim} wrapMode="none">{" STATE   "}</text>
        </box>
        <Lines rows={lines} selected={selected} theme={theme} onPress={onMove} rail />
      </Panel>
    </box>
  )
}

function RefusedPane({ body, theme, onMove, send }: { body: Rec; theme: Theme; onMove: (index: number) => void; send: Send }) {
  const status = rec(body.status)
  const refused = rows(status.refused)
  const running = flag(status.running)
  const lines: Line[] = refused.map((row) => ({ label: str(row.name), detail: str(row.reason), color: "warn" }))
  return (
    <Panel title={`Refused descriptors ${refused.length}`} hot theme={theme} actions={<Action label="Refresh (u)" theme={theme} onPress={() => sendKey(send, { Char: "u" })} />}>
      {refused.length
        ? <Lines rows={lines} selected={num(body.refused_selected)} theme={theme} onPress={onMove} rail />
        : <box paddingLeft={1}><text fg={running ? theme.success : theme.dim}>{running ? "● None. Every descriptor passed the handshake." : "○ No data. Start the daemon to run the handshake."}</text></box>}
    </Panel>
  )
}
