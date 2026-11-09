import type { ReactNode } from "react"
import { useState } from "react"
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
  ready: { glyph: "●", text: "ready", reason: "image active", role: "success" },
  local: { glyph: "▒", text: "local", reason: "pulled, not active", role: "info" },
  remote: { glyph: "·", text: "remote", reason: "not pulled", role: "dim" },
}
const readiness = (mark: string) => READINESS[mark] ?? READINESS.remote!

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
      {tiles ? <StatusBoard body={body} theme={theme} /> : <StatusStrip body={body} theme={theme} />}
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

type Gauge = { title: string; up: boolean | null; value: string; detail: string; bar?: number }

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
    { title: "Guest", up: !!active, value: active ? str(active.id) : "none", detail: `${images.filter((row) => flag(row.local)).length}/${images.length} images pulled` },
    { title: "Boundary", up: flag(status.boundary_up), value: flag(status.boundary_up) ? "up" : "down", detail: flag(status.boundary_up) ? "guest isolated" : "no boundary guest" },
    { title: "Egress", up: net === "off" || net === "scope" ? true : net === "down" ? false : null, value: net, detail: `${rules} allow rule${rules === 1 ? "" : "s"}` },
    { title: "Catalog", up: ready > 0, value: `${ready}/${total}`, detail: "", bar: total ? ready / total : 0 },
  ]
}

const gaugeGlyph = (up: boolean | null) => up == null ? "◌" : up ? "●" : "○"
const gaugeColor = (theme: Theme, up: boolean | null) => up == null ? theme.dim : up ? theme.success : theme.warn

function StatusBoard({ body, theme }: { body: Rec; theme: Theme }) {
  return (
    <box flexDirection="row" flexShrink={0} height={4} columnGap={1} minWidth={0}>
      {gauges(body).map((gauge) => (
        <box key={gauge.title} border borderStyle="rounded" borderColor={theme.dim} title={` ${gauge.title} `} titleColor={theme.dim} flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden" flexDirection="column" paddingLeft={1} paddingRight={1}>
          <text wrapMode="none"><span fg={gaugeColor(theme, gauge.up)}>{`${gaugeGlyph(gauge.up)} `}</span><span fg={theme.fg}>{gauge.value}</span></text>
          {gauge.bar != null ? <Bar ratio={gauge.bar} theme={theme} /> : <text wrapMode="none" fg={theme.dim}>{gauge.detail}</text>}
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
    <Panel title="Operations" maxLines={3} hot={false} theme={theme}>
      <ControlRow label="YOLO" value={armed ? "armed" : "locked"} color={armed ? theme.warn : theme.dim} theme={theme} onPress={() => sendKey(send, { Char: "o" })} control={<><Toggle checked={armed} theme={theme} /><text fg={theme.dim}> (o) </text></>} />
      <ControlRow label="Mode" value={locked && !yolo ? `${mode} · needs daemon + arm` : mode} color={yolo ? theme.error : theme.fg} theme={theme} control={<Action label={yolo ? "Leave (e)" : "Enter (e)"} plain disabled={locked} theme={theme} onPress={() => sendKey(send, { Char: "e" })} />} />
      <ControlRow label="Lane" value={str(body.lane_label) || "none"} theme={theme} control={<Action label="Cycle (l)" plain theme={theme} onPress={() => sendKey(send, { Char: "l" })} />} />
    </Panel>
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
      <Action label={`Lane ${str(body.lane_label) || "none"} (l)`} plain theme={theme} onPress={key("l")} />
    </box>
  )
}

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
    const cells = [str(row.id).padEnd(idWidth), str(row.image).padEnd(imageWidth), ...(wideCols ? [str(row.exec).padEnd(6)] : []), str(row.blurb)]
    return { label: cells.join(" "), split: true, badge: { text: `${state.glyph} ${state.text}`.padEnd(8), role: state.role } }
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
  const ops = toolOps(body, tool)
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
            <text wrapMode="none"><span fg={theme[state.role]}>{`${state.glyph} `}</span><span fg={theme.accent}>{str(tool.id)}</span><span fg={theme.dim}>{`  image ${str(tool.image) || "—"} · exec ${str(tool.exec) || "—"} · ${state.reason}`}</span></text>
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
  const images = rows(body.images)
  const selected = num(body.images_selected)
  const active = images.find((row) => flag(row.active))
  const pulled = images.filter((row) => flag(row.local)).length
  const sizeWidth = 8
  const idWidth = Math.min(16, Math.max(10, ...images.map((row) => str(row.id).length + 1)))
  const lines: Line[] = images.map((row) => {
    const role: Role = flag(row.active) ? "success" : flag(row.local) ? "info" : "dim"
    const text = flag(row.active) ? "● active" : flag(row.local) ? "▒ local" : "· remote"
    return { label: `${str(row.id).padEnd(idWidth)} ${(str(row.size_label) || "—").padEnd(sizeWidth)} ${str(row.blurb)}`, split: true, badge: { text: text.padEnd(8), role } }
  })
  const key = (c: string) => () => sendKey(send, { Char: c })
  const actions = images[selected] ? (
    <>
      <Action label="Activate (enter)" primary theme={theme} onPress={() => sendKey(send, "Enter")} />
      <Action label="Pull (p)" theme={theme} onPress={key("p")} />
      <Action label="Remove (x)" theme={theme} onPress={key("x")} />
      {width >= 70 ? <Action label="Refresh (r)" theme={theme} onPress={key("r")} /> : null}
    </>
  ) : <Action label="Refresh (r)" theme={theme} onPress={key("r")} />
  return (
    <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0}>
      <text flexShrink={0} wrapMode="none">
        <span fg={active ? theme.success : theme.dim}>{active ? "● " : "○ "}</span>
        <span fg={theme.dim}>active </span><span fg={theme.fg}>{active ? str(active.id) : "none"}</span>
        <span fg={theme.dim}>{`   pulled ${pulled}/${images.length}`}</span>
      </text>
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
