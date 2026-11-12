import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import type { SyntaxStyle } from "@opentui/core"
import { useKeyboard, useTerminalDimensions } from "@opentui/react"
import { scrollbars, type Theme } from "./theme.ts"
import { Action, Panel } from "./modes/bits.tsx"
import { MarkdownBody } from "./chat.tsx"
import type { Goal, GoalNode, Report } from "./room-link.ts"
import { around, fields, goalRows, press, reportRows, shelfRows, windowOf, type TreeRow } from "./room-tree.ts"

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
  useKeyboard((key) => {
    if (key.eventType === "release") return
    const name = key.name
    if (detail) {
      if (name === "escape" || name === "left" || name === "backspace" || name === "q") { setTrail((t) => t.slice(0, -1)); setPick(0); return }
      if (name === "up" || name === "k") setPick((p) => Math.max(0, p - 1))
      else if (name === "down" || name === "j") setPick((p) => Math.min(Math.max(0, links.length - 1), p + 1))
      else if ((name === "return" || name === "enter" || name === "right") && links[pick]) follow(links[pick]!.to)
      return
    }
    if (name === "escape" || name === "q") { onBack(); return }
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
    ? (links.length ? "↑↓ choose a link · enter follows it · esc goes back" : "esc goes back")
    : "↑↓ move · → or enter opens · ← closes · click a folder to fold it, a line to open it · esc closes"

  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1} paddingBottom={1}>
      {header}
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} paddingTop={1}>
        {detail ? (
          <DetailView detail={detail} data={data} item={item} links={links} pick={pick} onPick={setPick} onFollow={follow} theme={theme} syntaxStyle={syntaxStyle} width={width} />
        ) : (
          <Panel title={TITLES[view]} hot={false} theme={theme} status={data ? summary(view, data) : "loading…"}>
            {rows.length === 0
              ? <text fg={theme.dim} paddingLeft={1}>{data ? EMPTY[view] : "Loading…"}</text>
              : <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
                  {shown.rows.map((row) => (
                    <Line key={row.id} row={row} on={row.id === selected} theme={theme} width={width - 8}
                      onPress={() => {
                        setSelected(row.id)
                        if (row.kind === "leaf") { open(row); return }
                        setCollapsed((c) => { const n = new Set(c); if (n.has(row.id)) n.delete(row.id); else n.add(row.id); return n })
                      }} />
                  ))}
                </box>}
          </Panel>
        )}
      </box>
      <text fg={theme.dim} wrapMode="none">{` ${hint}`}</text>
    </box>
  )
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

function Line({ row, on, theme, width, onPress }: { row: TreeRow; on: boolean; theme: Theme; width: number; onPress: () => void }) {
  const glyph = row.kind === "folder" ? (row.open ? "▾" : "▸") : "•"
  const tone = MARKS[row.mark.toLowerCase()]
  const mark = row.mark ? (row.kind === "folder" ? `(${row.mark})` : row.mark) : ""
  const room = Math.max(10, width - row.depth * 2 - 4 - (mark ? mark.length + 1 : 0))
  const text = row.detail ? `${row.label}  ${row.detail}` : row.label
  const clipped = [...text].length > room ? `${[...text].slice(0, room - 1).join("")}…` : text
  const labelLen = Math.min([...row.label].length, [...clipped].length)
  const label = [...clipped].slice(0, labelLen).join("")
  const rest = [...clipped].slice(labelLen).join("")
  return (
    <box height={1} flexShrink={0} flexDirection="row" backgroundColor={on ? theme.panel : undefined} paddingLeft={1} onMouseDown={onPress}>
      <text wrapMode="none" fg={theme.fg}>
        <span fg={on ? theme.accent : theme.dim}>{`${"  ".repeat(row.depth)}${glyph} `}</span>
        <span fg={row.kind === "folder" ? theme.accent : theme.fg}>{label}</span>
        <span fg={theme.dim}>{rest}</span>
        {mark ? <span fg={tone ? theme[tone] : theme.dim}>{` ${mark}`}</span> : null}
      </text>
    </box>
  )
}

function linksOf(detail: Detail, data: BrowseData | null, item: OpenItem | null): Link[] {
  if (!data) return []
  if (detail.type === "node") {
    const goal = data.goals.find((g) => g.goal_key === detail.goal)
    const node = goal?.nodes.find((n) => n.id === detail.node)
    if (!goal || !node) return []
    const near = around(goal, node.id)
    const out: Link[] = []
    if (node.resource_key) out.push({ label: node.resource_key, hint: "the shelf item", to: { type: "item", key: node.resource_key } })
    for (const e of near.from) out.push({ label: e.node.label || e.node.id, hint: `${e.rel || "leads"} → this`, to: { type: "node", goal: goal.goal_key, node: e.node.id } })
    for (const e of near.to) out.push({ label: e.node.label || e.node.id, hint: `this ${e.rel || "leads"} →`, to: { type: "node", goal: goal.goal_key, node: e.node.id } })
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

function DetailView({ detail, data, item, links, pick, onPick, onFollow, theme, syntaxStyle, width }: {
  detail: Detail; data: BrowseData | null; item: OpenItem | null; links: Link[]; pick: number
  onPick: (i: number) => void; onFollow: (to: Detail) => void; theme: Theme; syntaxStyle: SyntaxStyle; width: number
}) {
  let body: ReactNode = null
  let title = ""
  if (detail.type === "node") {
    const goal = data?.goals.find((g) => g.goal_key === detail.goal)
    const node = goal?.nodes.find((n) => n.id === detail.node)
    title = node ? node.label || node.id : detail.node
    body = node ? <NodeBody node={node} goal={goal!} theme={theme} syntaxStyle={syntaxStyle} /> : <text fg={theme.dim}>This node is no longer on the map.</text>
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
        <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
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

function NodeBody({ node, goal, theme, syntaxStyle }: { node: GoalNode; goal: Goal; theme: Theme; syntaxStyle: SyntaxStyle }) {
  const rows: [string, string][] = [["goal", goal.goal_key], ["kind", node.kind], ["status", node.status], ["severity", node.severity], ["shelf item", node.resource_key]]
  return (
    <>
      {rows.filter(([, v]) => v).map(([name, value]) => <Field key={name} name={name} value={value} theme={theme} syntaxStyle={syntaxStyle} />)}
      {node.note ? <Field name="note" value={node.note} theme={theme} syntaxStyle={syntaxStyle} /> : null}
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
