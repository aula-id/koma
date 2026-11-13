import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import type { ScrollBoxRenderable, SyntaxStyle } from "@opentui/core"
import { useKeyboard, useTerminalDimensions } from "@opentui/react"
import { scrollbars, type Theme } from "./theme.ts"
import { Action, Panel } from "./modes/bits.tsx"
import { MarkdownBody } from "./chat.tsx"
import type { Goal, GoalNode, Report } from "./room-link.ts"
import { describe, fields, goalRows, press, reportRows, shelfRows, windowOf, type TreeRow } from "./room-tree.ts"
import { boxAt, drawGraph, layoutGraph, pan, readingOrder, reveal, windowLines, type Tone } from "./room-graph.ts"

export type BrowseView = "shelf" | "trajectory" | "report"
export type OpenItem = { key: string; revision: number; updated: number; content: string }
export type BrowseData = { shelf: import("./room-link.ts").ShelfRow[]; goals: Goal[]; reports: Report[] }

/** Where the user is below the list: one item, one goal node, one report. */
export type Detail = { type: "item"; key: string } | { type: "node"; goal: string; node: string } | { type: "report"; key: string }

/** A line the user can follow from a detail screen. */
type Link = { label: string; hint: string; to: Detail }

const TITLES: Record<BrowseView, string> = { shelf: "shelf", trajectory: "trajectory", report: "reports" }
const EMPTY: Record<BrowseView, string> = {
  shelf: "The shelf is empty. Findings appear here as the specialists publish them.",
  trajectory: "No trajectory yet. Goals appear here when the Chief hands work out.",
  report: "No report yet. The reporter publishes one once findings are confirmed.",
}

const PREFIXES = ["claims/", "evidence/", "verdicts/", "findings/", "reports/", "assets/", "scope/", "trajectory/", "results/", "notes/", "skills/"]
const isKey = (text: string) => PREFIXES.some((p) => text.startsWith(p)) && !/\s/.test(text)

/**
 * The browse screens: a folder tree you can open and close, and a detail screen for what you
 * pick. Keys: arrows move, Right or Enter opens, Left closes or climbs, Esc goes back.
 */
export function BrowseScreen({ view, theme, syntaxStyle, data, item, onOpenItem, onRefresh, onBack }: {
  view: BrowseView
  theme: Theme
  syntaxStyle: SyntaxStyle
  data: BrowseData | null
  item: OpenItem | null
  onOpenItem: (key: string) => void
  onRefresh: () => void
  onBack: () => void
}) {
  const { height, width } = useTerminalDimensions()
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [selected, setSelected] = useState("")
  const [start, setStart] = useState(0)
  const [trail, setTrail] = useState<Detail[]>([])
  const [pick, setPick] = useState(0)
  const detail = trail.at(-1) ?? null
  // The trajectory has a second way to look: the graph, drawn in boxes you can scroll across.
  const [mode, setMode] = useState<"tree" | "graph">("tree")
  const [goalAt, setGoalAt] = useState(0)
  const [boxId, setBoxId] = useState<string | null>(null)
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  const canvas = useRef<import("@opentui/core").BoxRenderable | null>(null)
  const graphOn = view === "trajectory" && mode === "graph"
  const goal = data?.goals[Math.min(goalAt, Math.max(0, (data?.goals.length ?? 1) - 1))] ?? null
  const layout = useMemo(() => (goal ? layoutGraph(goal) : null), [goal])
  const grid = useMemo(() => (layout ? drawGraph(layout, boxId) : []), [layout, boxId])

  // Fresh data while the screen is open: findings arrive as the lanes work.
  useEffect(() => {
    onRefresh()
    const timer = setInterval(onRefresh, 3000)
    return () => clearInterval(timer)
  }, [view])

  const rows = useMemo<TreeRow[]>(() => {
    if (!data) return []
    return view === "shelf" ? shelfRows(data.shelf, collapsed) : view === "trajectory" ? goalRows(data.goals, collapsed) : reportRows(data.reports)
  }, [data, view, collapsed])

  const listHeight = Math.max(3, height - 9)
  const viewSize = { width: Math.max(10, width - 8), height: listHeight }
  const shown = windowOf(rows, selected, start, listHeight)
  useEffect(() => { if (shown.from !== start) setStart(shown.from) }, [shown.from])
  useEffect(() => { if (!selected && rows[0]) setSelected(rows[0].id) }, [rows, selected])

  const open = (row: TreeRow) => {
    if (!row.ref) return
    const to: Detail = row.ref.type === "item" ? { type: "item", key: row.ref.key } : row.ref.type === "node" ? { type: "node", goal: row.ref.goal, node: row.ref.node } : { type: "report", key: row.ref.key }
    follow(to)
  }
  const follow = (to: Detail) => {
    if (to.type === "item") onOpenItem(to.key)
    // A node stands for a shelf item; its content is the detail, so fetch it with the node.
    if (to.type === "node") {
      const key = data?.goals.find((g) => g.goal_key === to.goal)?.nodes.find((n) => n.id === to.node)?.resource_key
      if (key) onOpenItem(key)
    }
    setPick(0)
    setTrail((t) => [...t, to])
  }
  const step = (key: string) => {
    const next = press(rows, selected, collapsed, key)
    setSelected(next.selected)
    setCollapsed(next.collapsed)
    if (next.open) open(next.open)
  }

  const links = detail ? linksOf(detail, data, item) : []
  const bodyRef = useRef<ScrollBoxRenderable | null>(null)
  // A new detail starts at its top.
  useEffect(() => { bodyRef.current?.scrollTo(0) }, [trail.length, item?.key])
  useKeyboard((key) => {
    if (key.eventType === "release") return
    const name = key.name
    if (detail) {
      if (name === "escape" || name === "left" || name === "backspace" || name === "q") { setTrail((t) => t.slice(0, -1)); setPick(0); return }
      const box = bodyRef.current
      const page = Math.max(1, (box?.viewport.height ?? 10) - 1)
      if (box && (name === "pagedown" || name === "space")) { box.scrollBy(page); return }
      if (box && name === "pageup") { box.scrollBy(-page); return }
      if (box && name === "home") { box.scrollTo(0); return }
      if (box && name === "end") { box.scrollTo(box.scrollHeight); return }
      if (name === "up" || name === "k") setPick((p) => Math.max(0, p - 1))
      else if (name === "down" || name === "j") setPick((p) => Math.min(Math.max(0, links.length - 1), p + 1))
      else if ((name === "return" || name === "enter" || name === "right") && links[pick]) follow(links[pick]!.to)
      return
    }
    if (name === "escape" || name === "q") { onBack(); return }
    if (view === "trajectory" && name === "g" && goal && layout) {
      // Start the graph at what the tree had selected.
      const node = selected.startsWith("node:") ? selected.split(":").slice(2).join(":") : null
      const sel = node && layout.boxes.some((b) => b.id === node) ? node : readingOrder(layout)[0]?.id ?? null
      setBoxId(sel)
      const box = layout.boxes.find((b) => b.id === sel)
      setOffset(box ? reveal({ x: 0, y: 0 }, box, layout, viewSize) : { x: 0, y: 0 })
      setMode("graph")
      return
    }
    if (graphOn && layout) {
      const move = (dx: number, dy: number) => setOffset((o) => pan(o, dx, dy, layout, viewSize))
      const cycle = (by: number) => {
        const order = readingOrder(layout)
        if (!order.length) return
        const at = Math.max(0, order.findIndex((b) => b.id === boxId))
        const next = order[(at + by + order.length) % order.length]!
        setBoxId(next.id)
        setOffset((o) => reveal(o, next, layout, viewSize))
      }
      if (name === "t" || name === "g") setMode("tree")
      else if (name === "left" || name === "h") move(-6, 0)
      else if (name === "right" || name === "l") move(6, 0)
      else if (name === "up" || name === "k") move(0, -2)
      else if (name === "down" || name === "j") move(0, 2)
      else if (name === "pageup") move(0, -viewSize.height)
      else if (name === "pagedown") move(0, viewSize.height)
      else if (name === "home") setOffset({ x: 0, y: 0 })
      else if (name === "end") move(layout.width, layout.height)
      else if (name === "tab") cycle(key.shift ? -1 : 1)
      else if (name === "n") cycle(1)
      else if (name === "p") cycle(-1)
      else if (name === "]" || name === "}") { setGoalAt((g) => Math.min((data?.goals.length ?? 1) - 1, g + 1)); setBoxId(null); setOffset({ x: 0, y: 0 }) }
      else if (name === "[" || name === "{") { setGoalAt((g) => Math.max(0, g - 1)); setBoxId(null); setOffset({ x: 0, y: 0 }) }
      else if ((name === "return" || name === "enter") && boxId && goal) follow({ type: "node", goal: goal.goal_key, node: boxId })
      return
    }
    step(name === "space" ? "space" : name)
  })

  const header = (
    <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
      <text flexShrink={0}>
        <span fg={theme.fg}>Koma</span>
        <span fg={theme.dim}> | </span>
        <span fg={theme.accent}>{`secure room · ${TITLES[view]}`}</span>
        {detail ? <span fg={theme.dim}>{` · ${crumb(detail)}`}</span> : null}
      </text>
      <Action label={detail ? "Back (esc)" : "Close (esc)"} theme={theme} plain onPress={() => detail ? (setTrail((t) => t.slice(0, -1)), setPick(0)) : onBack()} />
    </box>
  )
  const hint = detail
    ? (links.length ? "↑↓ choose a link · enter follows it · pgup/pgdn scroll · esc goes back" : "pgup/pgdn scroll · esc goes back")
    : graphOn
      ? "←→↑↓ scroll · pgup/pgdn · tab next node · enter opens · [ ] goal · t tree · esc closes"
      : view === "trajectory"
        ? "↑↓ move · enter opens · space or ←→ folds · g graph · click the arrow to fold, the line to open · esc closes"
        : "↑↓ move · → or enter opens · ← closes · click a folder to fold it, a line to open it · esc closes"

  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1} paddingBottom={1}>
      {header}
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} paddingTop={1}>
        {detail ? (
          <DetailView bodyRef={bodyRef} detail={detail} data={data} item={item} links={links} pick={pick} onPick={setPick} onFollow={follow} theme={theme} syntaxStyle={syntaxStyle} width={width} />
        ) : (
          graphOn ? (
            <Panel title="trajectory · graph" hot={false} theme={theme} status={layout && goal ? `${goal.goal_key} ${goalAt + 1}/${data?.goals.length ?? 1} · x ${offset.x}/${Math.max(0, layout.width - viewSize.width)} · y ${offset.y}/${Math.max(0, layout.height - viewSize.height)}` : ""}>
              {!layout || layout.boxes.length === 0 ? <text fg={theme.dim} paddingLeft={1}>{goal ? "This goal has no steps yet." : EMPTY.trajectory}</text> : (
                <box
                  ref={canvas}
                  flexDirection="column"
                  flexGrow={1}
                  flexBasis={0}
                  minHeight={0}
                  paddingLeft={1}
                  onMouseScroll={(event: { scroll?: { direction?: string } }) => {
                    const d = event.scroll?.direction
                    setOffset((o) => pan(o, d === "left" ? -6 : d === "right" ? 6 : 0, d === "up" ? -2 : d === "down" ? 2 : 0, layout, viewSize))
                  }}
                  onMouseDown={(event: { x: number; y: number }) => {
                    const c = canvas.current
                    if (!c) return
                    const hit = boxAt(layout, event.x - c.x - 1 + offset.x, event.y - c.y + offset.y)
                    if (!hit) return
                    setBoxId(hit.id)
                    if (goal) follow({ type: "node", goal: goal.goal_key, node: hit.id })
                  }}
                >
                  {windowLines(grid, offset, viewSize).map((runs, row) => (
                    <text key={row} wrapMode="none" fg={theme.dim}>
                      {runs.map((run, i) => <span key={i} fg={graphInk(run.tone, theme)}>{run.text}</span>)}
                    </text>
                  ))}
                </box>
              )}
            </Panel>
          ) : (
          <Panel title={TITLES[view]} hot={false} theme={theme} status={data ? summary(view, data) : "loading…"}>
            {rows.length === 0
              ? <text fg={theme.dim} paddingLeft={1}>{data ? EMPTY[view] : "Loading…"}</text>
              : <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
                  {shown.rows.map((row) => (
                    <Line key={row.id} row={row} on={row.id === selected} theme={theme} width={width - 8}
                      onFold={() => {
                        setSelected(row.id)
                        setCollapsed((c) => { const n = new Set(c); if (n.has(row.id)) n.delete(row.id); else n.add(row.id); return n })
                      }}
                      onPress={() => {
                        setSelected(row.id)
                        if (row.ref) { open(row); return }
                        setCollapsed((c) => { const n = new Set(c); if (n.has(row.id)) n.delete(row.id); else n.add(row.id); return n })
                      }} />
                  ))}
                </box>}
          </Panel>
          )
        )}
      </box>
      <text fg={theme.dim} wrapMode="none">{` ${hint}`}</text>
    </box>
  )
}

function graphInk(tone: Tone, theme: Theme): string {
  switch (tone) {
    case "select": return theme.accent
    case "success": return theme.success
    case "warn": return theme.warn
    case "error": return theme.error
    case "info": return theme.info
    case "box": return theme.fg
    default: return theme.dim
  }
}

function summary(view: BrowseView, data: BrowseData): string {
  if (view === "shelf") return `${data.shelf.length} items`
  if (view === "trajectory") return `${data.goals.length} goals`
  return `${data.reports.length} reports`
}

function crumb(detail: Detail): string {
  return detail.type === "node" ? `${detail.goal} / ${detail.node}` : detail.key
}

const MARKS: Record<string, "error" | "warn" | "info" | "success"> = { critical: "error", high: "error", medium: "warn", low: "info", info: "info", open: "warn", confirmed: "success", verified: "success", returned: "warn", dismissed: "info", "no-go": "error", go: "success" }

function Line({ row, on, theme, width, onPress, onFold }: { row: TreeRow; on: boolean; theme: Theme; width: number; onPress: () => void; onFold: () => void }) {
  const glyph = row.kind === "folder" ? (row.open ? "▾" : "▸") : "•"
  const tone = MARKS[row.mark.toLowerCase()]
  const mark = row.mark ? (row.kind === "folder" && !row.ref ? `(${row.mark})` : row.mark) : ""
  const front = `${row.guide ?? "  ".repeat(row.depth)}${glyph} `
  const number = row.number ? `(${row.number}) ` : ""
  const refs = row.refs?.length ? ` [${row.refs.map((r) => `(${r})`).join(", ")}]` : ""
  const room = Math.max(10, width - [...front].length - number.length - (mark ? mark.length + 1 : 0) - refs.length)
  const text = row.detail ? `${row.label}  ${row.detail}` : row.label
  const clipped = [...text].length > room ? `${[...text].slice(0, room - 1).join("")}…` : text
  const labelLen = Math.min([...row.label].length, [...clipped].length)
  const label = [...clipped].slice(0, labelLen).join("")
  const rest = [...clipped].slice(labelLen).join("")
  return (
    <box height={1} flexShrink={0} flexDirection="row" backgroundColor={on ? theme.panel : undefined} paddingLeft={1}>
      <box flexShrink={0} onMouseDown={onFold}>
        <text wrapMode="none" fg={on ? theme.accent : theme.dim}>{front}</text>
      </box>
      <box flexGrow={1} minWidth={0} onMouseDown={onPress}>
        <text wrapMode="none" fg={theme.fg}>
          {number ? <span fg={theme.dim}>{number}</span> : null}
          <span fg={row.kind === "folder" && !row.ref ? theme.accent : theme.fg}>{label}</span>
          <span fg={theme.dim}>{rest}</span>
          {mark ? <span fg={tone ? theme[tone] : theme.dim}>{` ${mark}`}</span> : null}
          {refs ? <span fg={theme.info}>{refs}</span> : null}
        </text>
      </box>
    </box>
  )
}

function linksOf(detail: Detail, data: BrowseData | null, item: OpenItem | null): Link[] {
  if (!data) return []
  if (detail.type === "node") {
    const goal = data.goals.find((g) => g.goal_key === detail.goal)
    const node = goal?.nodes.find((n) => n.id === detail.node)
    if (!goal || !node) return []
    const near = describe(goal, node.id)
    const out: Link[] = []
    const to = (n: { id: string }): Detail => ({ type: "node", goal: goal.goal_key, node: n.id })
    if (node.resource_key) out.push({ label: node.resource_key, hint: "the shelf item", to: { type: "item", key: node.resource_key } })
    for (const e of near?.from ?? []) out.push({ label: `(${e.number}) ${e.node.label || e.node.id}`, hint: `reference from${e.rel ? ` · ${e.rel}` : ""}`, to: to(e.node) })
    for (const e of near?.to ?? []) out.push({ label: `(${e.number}) ${e.node.label || e.node.id}`, hint: `reference to${e.rel ? ` · ${e.rel}` : ""}`, to: to(e.node) })
    for (const e of near?.related ?? []) out.push({ label: `(${e.number}) ${e.node.label || e.node.id}`, hint: "related", to: to(e.node) })
    return out
  }
  if (detail.type === "report") {
    const report = data.reports.find((r) => r.key === detail.key)
    return (report?.finding_keys ?? []).map((k) => ({ label: k, hint: "finding", to: { type: "item", key: k } as Detail }))
  }
  if (!item || item.key !== detail.key) return []
  const list = fields(item.content) ?? []
  const keys = new Set<string>()
  for (const f of list) for (const part of f.value.split(/,\s*/)) if (isKey(part.trim()) && part.trim() !== item.key) keys.add(part.trim())
  return [...keys].map((k) => ({ label: k, hint: "linked", to: { type: "item", key: k } as Detail }))
}

function DetailView({ bodyRef, detail, data, item, links, pick, onPick, onFollow, theme, syntaxStyle, width }: {
  bodyRef: { current: ScrollBoxRenderable | null }
  detail: Detail; data: BrowseData | null; item: OpenItem | null; links: Link[]; pick: number
  onPick: (i: number) => void; onFollow: (to: Detail) => void; theme: Theme; syntaxStyle: SyntaxStyle; width: number
}) {
  let body: ReactNode = null
  let title = ""
  if (detail.type === "node") {
    const goal = data?.goals.find((g) => g.goal_key === detail.goal)
    const node = goal?.nodes.find((n) => n.id === detail.node)
    title = node ? node.label || node.id : detail.node
    body = node ? <NodeBody node={node} goal={goal!} item={item} theme={theme} syntaxStyle={syntaxStyle} /> : <text fg={theme.dim}>This node is no longer on the map.</text>
  } else if (detail.type === "report") {
    const report = data?.reports.find((r) => r.key === detail.key)
    title = report?.title || detail.key
    body = report ? <ReportBody report={report} theme={theme} syntaxStyle={syntaxStyle} /> : <text fg={theme.dim}>This report is no longer on the shelf.</text>
  } else {
    title = detail.key
    body = !item || item.key !== detail.key ? <text fg={theme.dim}>Loading…</text> : <ItemBody item={item} theme={theme} syntaxStyle={syntaxStyle} />
  }
  const shown = links.slice(Math.max(0, Math.min(pick - 3, links.length - 7)), Math.max(0, Math.min(pick - 3, links.length - 7)) + 7)
  const first = links.indexOf(shown[0]!)
  return (
    <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
      <Panel title={title.length > width - 20 ? `${title.slice(0, width - 21)}…` : title} hot={false} theme={theme}>
        <scrollbox ref={bodyRef} flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
          <box flexDirection="column" paddingLeft={1} paddingRight={1}>{body}</box>
        </scrollbox>
      </Panel>
      {links.length ? (
        <box flexShrink={0} flexDirection="column" paddingTop={1}>
          <text fg={theme.dim}>{` links (${links.length})`}</text>
          {shown.map((link, i) => (
            <box key={`${link.label}-${first + i}`} height={1} flexShrink={0} paddingLeft={1} backgroundColor={first + i === pick ? theme.panel : undefined} onMouseDown={() => { onPick(first + i); onFollow(link.to) }}>
              <text wrapMode="none"><span fg={first + i === pick ? theme.accent : theme.dim}>{first + i === pick ? "› " : "  "}</span><span fg={theme.accent}>{link.label}</span><span fg={theme.dim}>{`  ${link.hint}`}</span></text>
            </box>
          ))}
        </box>
      ) : null}
    </box>
  )
}

const LONG = new Set(["summary", "poc", "note", "detail", "body", "notes", "markdown"])

function Field({ name, value, theme, syntaxStyle }: { name: string; value: string; theme: Theme; syntaxStyle: SyntaxStyle }) {
  if (name === "markdown" || (name === "body" && value.includes("\n"))) {
    return <box flexDirection="column" paddingTop={1}><text fg={theme.dim}>{name}</text><MarkdownBody content={value} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} /></box>
  }
  if (LONG.has(name) || value.includes("\n") || value.length > 80) {
    return <box flexDirection="column" paddingTop={1}><text fg={theme.dim}>{name}</text><text fg={theme.fg} wrapMode="word">{value}</text></box>
  }
  return <text wrapMode="word"><span fg={theme.dim}>{`${name}  `}</span><span fg={MARKS[value.toLowerCase()] ? theme[MARKS[value.toLowerCase()]!] : theme.fg}>{value}</span></text>
}

function ItemBody({ item, theme, syntaxStyle }: { item: OpenItem; theme: Theme; syntaxStyle: SyntaxStyle }) {
  const list = fields(item.content)
  return (
    <>
      <text fg={theme.dim}>{`revision ${item.revision}`}</text>
      {list ? list.map((f) => <Field key={f.name} name={f.name} value={f.value} theme={theme} syntaxStyle={syntaxStyle} />)
        : <box paddingTop={1}><MarkdownBody content={item.content} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} /></box>}
    </>
  )
}

function NodeBody({ node, goal, item, theme, syntaxStyle }: { node: GoalNode; goal: Goal; item: OpenItem | null; theme: Theme; syntaxStyle: SyntaxStyle }) {
  const number = describe(goal, node.id)?.number ?? ""
  const rows: [string, string][] = [["number", number ? `(${number})` : ""], ["goal", goal.goal_key], ["kind", node.kind], ["status", node.status], ["severity", node.severity], ["shelf item", node.resource_key]]
  return (
    <>
      {rows.filter(([, v]) => v).map(([name, value]) => <Field key={name} name={name} value={value} theme={theme} syntaxStyle={syntaxStyle} />)}
      {node.note ? <Field name="note" value={node.note} theme={theme} syntaxStyle={syntaxStyle} /> : null}
      {node.resource_key ? (
        <box flexDirection="column" paddingTop={1}>
          <text fg={theme.dim}>{"── content ──"}</text>
          {item && item.key === node.resource_key ? <ItemBody item={item} theme={theme} syntaxStyle={syntaxStyle} /> : <text fg={theme.dim}>Loading…</text>}
        </box>
      ) : null}
    </>
  )
}

function ReportBody({ report, theme, syntaxStyle }: { report: Report; theme: Theme; syntaxStyle: SyntaxStyle }) {
  const rows: [string, string][] = [["audience", report.audience], ["go / no-go", report.go_no_go], ["severity", report.severity_summary], ["revision", `${report.revision}`]]
  return (
    <>
      {rows.filter(([, v]) => v).map(([name, value]) => <Field key={name} name={name} value={value} theme={theme} syntaxStyle={syntaxStyle} />)}
      <box paddingTop={1} flexDirection="column"><MarkdownBody content={report.markdown || "_The report has no body yet._"} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} /></box>
    </>
  )
}
