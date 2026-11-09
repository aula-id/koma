import { useContext, useEffect, useRef, useState, useSyncExternalStore } from "react"
import type { BoxRenderable, ScrollBoxRenderable } from "@opentui/core"
import { UsageContext } from "../usage-reader.ts"
import { useKeyboard, useRenderer } from "@opentui/react"
import { scrollbars, type Theme } from "../theme.ts"
import { Action, InputSuspendedContext } from "./bits.tsx"
import { num, rec, rows, str, type Rec } from "./read.ts"

export function money(value: number): string {
  if (!Number.isFinite(value)) return "$0.0000"
  if (Math.abs(value) >= 100) return `$${value.toFixed(2)}`
  return `$${value.toFixed(4)}`
}

export function groupInt(value: number): string {
  const n = Math.round(value)
  const sign = n < 0 ? "-" : ""
  return sign + String(Math.abs(n)).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
}

export function pct(part: number, whole: number): string {
  if (!(whole > 0)) return "0.0%"
  return `${((part / whole) * 100).toFixed(1)}%`
}

/** Sum a long ledger into the columns that fit the screen, keeping time order. */
export function fitSeries(values: number[], epochs: number[], maxCols: number): { values: number[]; epochs: number[] } {
  const cols = Math.max(1, maxCols)
  if (values.length <= cols) return { values, epochs }
  const outValues: number[] = []
  const outEpochs: number[] = []
  for (let i = 0; i < cols; i++) {
    const start = Math.floor((i * values.length) / cols)
    const end = Math.floor(((i + 1) * values.length) / cols)
    let sum = 0
    for (let j = start; j < end; j++) sum += values[j] ?? 0
    outValues.push(sum)
    outEpochs.push(epochs[start] ?? 0)
  }
  return { values: outValues, epochs: outEpochs }
}

/** One glyph per column, from the top row down. The tallest value is a full bar. */
export function chartCells(values: number[], rows: number): string[][] {
  const height = Math.max(1, rows)
  const max = Math.max(0, ...values)
  return values.map((value) => {
    const filled = max <= 0 || value <= 0 ? 0 : Math.max(1, Math.round((value / max) * height))
    return Array.from({ length: height }, (_, row) => (height - 1 - row < filled ? "█" : " "))
  })
}

export function barLine(value: number, max: number, width: number): string {
  const size = Math.max(1, width)
  if (!(max > 0) || value <= 0) return "·".repeat(size)
  const filled = Math.max(1, Math.min(size, Math.round((value / max) * size)))
  return `${"█".repeat(filled)}${"·".repeat(size - filled)}`
}

export type Period = { start: number; end: number; cost: number; tokens: number; future: boolean }
export function periods(buckets: Rec[], start: number, end: number, step: number, offset: number, now: number): Period[] {
  const first = Math.floor((start + offset) / step) * step - offset
  const byTime = new Map(buckets.map(b => [num(b.bucket_epoch), b]))
  const result: Period[] = []
  for (let epoch = first; epoch < end; epoch += step) {
    const row = byTime.get(epoch)
    result.push({ start: Math.max(start, epoch), end: Math.min(end, epoch + step), cost: num(row?.cost), tokens: num(row?.tokens), future: epoch > now })
  }
  return result
}
export function foldPeriods(points: Period[], count: number): Period[] {
  count = Math.max(1, count)
  if (points.length <= count) return points
  return Array.from({ length: count }, (_, i) => {
    const group = points.slice(Math.floor(i * points.length / count), Math.floor((i + 1) * points.length / count))
    return { start: group[0]!.start, end: group.at(-1)!.end, cost: group.reduce((s, p) => s + p.cost, 0), tokens: group.reduce((s, p) => s + p.tokens, 0), future: group.every(p => p.future) }
  })
}
export function UsageBoard({ body, theme, send = () => {} }: { body: Rec; theme: Theme; send?: (request: unknown) => void }) {
  const renderer = useRenderer()
  const ref = useRef<BoxRenderable | null>(null)
  const [width, setWidth] = useState(renderer.width)
  useEffect(() => {
    const read = () => { const next = Math.max(1, Math.floor(ref.current?.width ?? renderer.width)); setWidth(current => current === next ? current : next) }
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer])
  const scroll = useRef<ScrollBoxRenderable | null>(null)
  const suspended = useContext(InputSuspendedContext)
  const context = useContext(UsageContext)
  const empty = useRef({ result: null, refreshing: false, error: "", missing: false }).current
  const state = useSyncExternalStore(context?.reader.subscribe ?? (() => () => {}), () => context?.reader.state ?? empty)
  const session = body.view === "session", range = str(body.range).toLowerCase() || "today"
  const metric = body.metric === "tokens" ? "tokens" : "cost"
  useEffect(() => {
    context?.reader.select({ scope: session ? "session" : "global", range, session_uuid: context.session })
    return () => context?.reader.select(null)
  }, [context?.reader, context?.session, session, range])
  const result = state.result && state.result.scope === (session ? "session" : "global") && state.result.range === range && state.result.session_uuid === context?.session ? state.result : null
  const data = rec(context ? result?.data : body.data)
  const models = rows(session ? data.session_models : data.top_models), heat = rows(session ? data.session_hourly : data.heatmap_buckets)
  const totals = session && !context ? {
    cost: models.reduce((s, r) => s + num(r.total_cost), 0), calls: num(data.session_calls),
    tokens_in: models.reduce((s, r) => s + num(r.tokens_in), 0), tokens_cached: models.reduce((s, r) => s + num(r.tokens_cached), 0), tokens_out: models.reduce((s, r) => s + num(r.tokens_out), 0),
  } : rec(data.totals)
  const cost = num(totals.cost), calls = num(totals.calls), input = num(totals.tokens_in), cached = num(totals.tokens_cached), output = num(totals.tokens_out)
  const offset = result?.utc_offset ?? -new Date().getTimezoneOffset() * 60, now = result?.collected_at ?? Date.now() / 1000
  const midnight = Math.floor((now + offset) / 86400) * 86400 - offset
  const start = result?.window_start ?? (session ? num(heat[0]?.bucket_epoch) || midnight : range === "today" ? midnight : now - (range === "week" ? 7 : 365) * 86400)
  const end = result?.window_end ?? (range === "today" && !session ? midnight + 86400 : now + 1)
  const year = !session && range === "year", side = width >= 116 && !year, chartWidth = side ? Math.floor((width - 1) * .4) : width - 1
  const points = periods(heat, start, end, year || !session && range === "week" ? 86400 : 3600, offset, now)
  const series = session ? foldPeriods(points, chartWidth - 4) : points
  const selectionKey = `${session}/${range}`
  const selections = useRef(new Map<string, number>())
  const [, redraw] = useState(0)
  const selectedAt = selections.current.get(selectionKey)
  let selected = series.findIndex(p => selectedAt != null && selectedAt >= p.start && selectedAt < p.end)
  if (selected < 0) selected = selectedAt != null && selectedAt < (series[0]?.start ?? 0) ? 0 : Math.max(0, series.findLastIndex(p => !p.future))
  const point = series[selected]
  if (point && selectedAt == null && (!context || result)) selections.current.set(selectionKey, point.start)
  const selectAt = (index: number) => {
    const p = series[Math.max(0, Math.min(series.length - 1, index))]
    if (p) { selections.current.set(selectionKey, p.start); redraw(n => n + 1) }
  }
  useKeyboard(key => {
    if (suspended || key.eventType === "release") return
    const delta = key.name === "left" ? year ? -7 : -1 : key.name === "right" ? year ? 7 : 1 : year && key.name === "up" ? -1 : year && key.name === "down" ? 1 : 0
    if (delta) { key.preventDefault(); selectAt(selected + delta) }
    if (key.name === "home") { key.preventDefault(); selectAt(0) }
    if (key.name === "end") { key.preventDefault(); selectAt(series.findLastIndex(p => !p.future)) }
    if (key.name === "pageup" || key.name === "pagedown") { key.preventDefault(); scroll.current?.scrollBy(key.name === "pageup" ? -1 : 1, "viewport") }
    if (key.name === "r") context?.reader.refresh()
  })
  const key = (code: unknown) => { if (!suspended) send({ SendKey: { code, mods: 0 } }) }
  const role = rec(data.role_split), live = rec(body.session_totals)
  const chart = <box border borderStyle="rounded" borderColor={theme.dim} title={` Activity / ${year ? "Year" : session ? "Session lifetime" : range} `} flexDirection="column" flexShrink={0} width={side ? chartWidth : "100%"} alignSelf="flex-start">
    {year ? <Calendar points={points} offset={offset} width={chartWidth - 2} selected={selected} metric={metric} theme={theme} onSelect={selectAt} /> : <Bars points={series} selected={selected} offset={offset} width={chartWidth - 2} metric={metric} theme={theme} onSelect={selectAt} />}
  </box>
  const table = <box border borderStyle="rounded" borderColor={theme.dim} title={` Models / ${models.length} `} flexDirection="column" flexGrow={side ? 1 : 0} flexShrink={0} minWidth={0}>
    <ModelTable models={models} width={side ? width - chartWidth - 3 : width - 3} theme={theme} metric={metric} total={metric === "cost" ? cost : input + output} />
    {!session ? <><text fg={theme.accent}>Roles · share of all recorded cost</text>{[["Main", "main"], ["Subagents", "sub"]].map(([label, roleKey]) => <text key={roleKey} fg={theme.fg} wrapMode="none">{`${label!.padEnd(10)} ${groupInt(num(role[`${roleKey}_calls`]))} calls  ${money(num(role[`${roleKey}_cost`]))}  ${pct(num(role[`${roleKey}_cost`]), cost)}`}</text>)}</> : null}
  </box>
  return <box ref={ref} flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} minWidth={0}>
    <box flexDirection="row" flexWrap="wrap" flexShrink={0}>
      <box flexDirection="row" flexShrink={0}>
        <Action label="Global" primary={!session} disabled={suspended} theme={theme} onPress={() => { if (session) key("Tab") }} />
        <text fg={theme.dim}>·</text>
        <Action label="Session (tab)" primary={session} disabled={suspended} theme={theme} onPress={() => { if (!session) key("Tab") }} />
        <text fg={theme.dim}>|</text>
      </box>
      {!session ? ["today", "week", "year"].map((r, i) => <box key={r} flexDirection="row" flexShrink={0}>
        <Action label={`${r[0]!.toUpperCase() + r.slice(1)} (${i + 1})`} primary={r === range} disabled={suspended} theme={theme} onPress={() => { if (r !== range) key({ Char: String(i + 1) }) }} />
        <text fg={theme.dim}>|</text>
      </box>) : null}
      <box flexDirection="row" flexShrink={0}>
        <Action label="Cost" primary={metric === "cost"} disabled={suspended} theme={theme} onPress={() => { if (metric !== "cost") key({ Char: "m" }) }} />
        <text fg={theme.dim}>·</text>
        <Action label="Tokens (m)" primary={metric === "tokens"} disabled={suspended} theme={theme} onPress={() => { if (metric !== "tokens") key({ Char: "m" }) }} />
        <text fg={theme.dim}>|</text>
      </box>
      <Action label="Back (esc)" disabled={suspended} theme={theme} onPress={() => key("Esc")} />
    </box>
    <box flexShrink={0} flexDirection="column">
      <text fg={theme.accent}>{`Summary · recorded ${money(cost)} · ${groupInt(calls)} calls`}</text>
      <text fg={theme.fg}>{`In ${groupInt(input)} · cached ${groupInt(cached)} (${pct(cached, input)}) · out ${groupInt(output)} · avg ${money(calls ? cost / calls : 0)}/call`}</text>
      {session && body.session_totals ? <text fg={theme.dim}>{`Live session ${money(num(live.cost))} · ${groupInt(num(live.tokens_in) + num(live.tokens_out))} tokens · ledger updates after calls`}</text> : null}
    </box>
    <scrollbox ref={scroll} flexGrow={1} flexBasis={0} minHeight={0} scrollbarOptions={scrollbars(theme)}>
      <box flexDirection={side ? "row" : "column"} gap={0} flexShrink={0} minWidth={0}>{chart}{table}</box>
    </scrollbox>
    <box height={width < 75 ? 2 : 1} flexShrink={0} flexDirection="column">
      <text fg={theme.fg} wrapMode="none">{point ? `${stamp(point.start, offset, year)} – ${stamp(point.end, offset, year)}${width >= 75 ? `  ${money(point.cost)} · ${groupInt(point.tokens)} tokens${point.future ? " · future" : ""}` : ""}` : "No recorded intervals"}</text>
      {width < 75 ? <text fg={theme.fg} wrapMode="none">{point ? `${money(point.cost)} · ${groupInt(point.tokens)} tokens${point.future ? " · future" : ""}` : ""}</text> : null}
    </box>
    <box height={1} flexShrink={0} flexDirection="row" justifyContent="space-between">
      <text flexGrow={1} flexBasis={0} minWidth={0} fg={state.error ? theme.warn : theme.dim} wrapMode="none">{!context ? "Snapshot refresh · older peer" : state.error ? `${state.result ? "Stale · " : ""}${state.error}` : state.refreshing ? "Refreshing…" : result ? `Collected ${stamp(result.collected_at, offset, false)} · UTC${offset >= 0 ? "+" : "−"}${Math.floor(Math.abs(offset) / 3600)}:${String(Math.abs(offset) / 60 % 60).padStart(2, "0")}` : "Loading ledger…"}</text>
      {context ? <Action label="Retry (r)" theme={theme} onPress={context.reader.refresh} /> : null}
    </box>
  </box>
}
/** Match the legacy chart's 33rd/66th/90th-percentile heat colors. */
export function heatLevels(values: number[]): number[] {
  const positive = values.filter(value => value > 0).sort((a, b) => a - b)
  const percentile = (percent: number) => positive[Math.floor((positive.length - 1) * percent / 100)] ?? 0
  const low = percentile(33), middle = percentile(66), high = percentile(90)
  return values.map(value => value <= 0 ? 0 : low >= high ? 2 : value <= low ? 1 : value <= middle ? 2 : value <= high ? 3 : 4)
}
function Bars({ points, selected, width, offset, metric, theme, onSelect }: { points: Period[]; selected: number; width: number; offset: number; metric: "cost" | "tokens"; theme: Theme; onSelect: (i: number) => void }) {
  const values = points.map(p => p[metric]), cells = chartCells(values, 5), levels = heatLevels(values)
  const start = Math.floor(selected * width / Math.max(1, points.length))
  const end = Math.floor((selected + 1) * width / Math.max(1, points.length))
  const markerWidth = Math.min(3, Math.max(1, width))
  const markerStart = Math.max(0, Math.min(width - markerWidth, Math.floor((start + end - markerWidth) / 2)))
  return <box flexDirection="column" flexShrink={0}>
    <box flexDirection="row" height={5} flexShrink={0}>{points.map((p, i) => {
      const cellWidth = Math.max(1, Math.floor((i + 1) * width / points.length) - Math.floor(i * width / points.length))
      const color = p.future ? theme.dim : theme.heat[levels[i]!]!
      return <box key={p.start} width={cellWidth} height={5} flexDirection="column" onMouseDown={() => onSelect(i)}>
        {cells[i]!.map((cell, row) => {
          const filled = !p.future && cell === "█"
          const glyph = p.future ? row === 4 ? "·" : " " : cell === " " && row === 4 ? "▁" : cell
          return <box key={row} height={1} flexShrink={0} backgroundColor={filled ? color : undefined}><text height={1} fg={color} bg={filled ? color : undefined}>{glyph.repeat(cellWidth)}</text></box>
        })}
      </box>
    })}</box>
    <text height={1} fg={theme.accent} wrapMode="none">{points.length ? " ".repeat(markerStart) + "^".repeat(markerWidth) : ""}</text>
    <box flexDirection="row" justifyContent="space-between"><text fg={theme.dim}>{points[0] ? stamp(points[0].start, offset, false) : "Empty"}</text><text fg={theme.dim}>{points.at(-1) ? stamp(points.at(-1)!.end, offset, false) : ""}</text></box>
  </box>
}
/** Fill every available calendar column; narrow views follow the selected week. */
export function calendarColumns(weeks: number, width: number, selectedWeek: number) {
  const available = Math.max(1, Math.floor(width) - 4)
  const visible = Math.max(1, Math.min(weeks, available))
  const from = Math.min(Math.max(0, selectedWeek - visible + 1), Math.max(0, weeks - visible))
  const starts = Array.from({ length: visible }, (_, i) => Math.floor(i * available / visible))
  const widths = starts.map((start, i) => Math.floor((i + 1) * available / visible) - start)
  return { from, visible, starts, widths, available }
}
function Calendar({ points, selected, width, offset, metric, theme, onSelect }: { points: Period[]; selected: number; width: number; offset: number; metric: "cost" | "tokens"; theme: Theme; onSelect: (i: number) => void }) {
  const first = points[0]?.start ?? 0, weekday = new Date((first + offset) * 1000).getUTCDay(), weeks = Math.ceil((points.length + weekday) / 7)
  const { from, visible, starts, widths, available } = calendarColumns(weeks, width, Math.floor((selected + weekday) / 7))
  const max = Math.max(0, ...points.map(p => p[metric]))
  const labels = Array.from({ length: available }, () => " ")
  const months: { position: number; label: string }[] = []
  for (let i = 0; i < visible; i++) {
    const index = (from + i) * 7 - weekday
    const d = new Date(((points[index]?.start ?? first + index * 86400) + offset + 6 * 86400) * 1000)
    if (i === 0 && d.getUTCDate() <= 21 || d.getUTCDate() <= 7) {
      const month = d.toLocaleString("en", { month: "short", timeZone: "UTC" })
      const position = Math.max(0, Math.min(starts[i]!, labels.length - 3))
      months.push({ position, label: month })
    }
  }
  for (let i = 0; i < months.length; i++) {
    const { position, label } = months[i]!
    // Omit a cramped partial first month rather than merging two month names.
    if (i === 0 && months[1] && months[1].position - position < 4) continue
    for (let j = 0; j < 3 && position + j < labels.length; j++) labels[position + j] = label[j]!
  }
  return <box flexDirection="column" flexShrink={0}>
    <text fg={theme.dim} wrapMode="none">{`    ${labels.join("")}`}</text>
    {Array.from({ length: 7 }, (_, day) => <box key={day} flexDirection="row" height={1} flexShrink={0}>
      <text width={4} fg={theme.dim}>{["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][day]}</text>
      {Array.from({ length: visible }, (_, col) => {
        const index = (from + col) * 7 + day - weekday, p = points[index], cellWidth = widths[col]!
        const level = !p || p[metric] <= 0 || max <= 0 ? 0 : Math.max(1, Math.ceil(p[metric] / max * 4))
        return <box key={col} width={cellWidth} height={1} onMouseDown={() => { if (p) onSelect(index) }}><text bg={index === selected ? theme.selBg : p ? theme.heat[level] : undefined} fg={index === selected ? theme.selFg : theme.dim}>{index === selected ? "◆".padEnd(cellWidth) : !p ? (index < 0 ? " " : "·").repeat(cellWidth) : p.future ? "·".repeat(cellWidth) : " ".repeat(cellWidth)}</text></box>
      })}
    </box>)}
    <text fg={theme.dim}>Less {theme.heat.map((color, i) => <span key={i} bg={color}>  </span>)} More{visible < weeks ? `  weeks ${from + 1}–${from + visible}/${weeks} ↔` : ""}</text>
  </box>
}
function ModelTable({ models, width, metric, total, theme }: { models: Rec[]; width: number; metric: "cost" | "tokens"; total: number; theme: Theme }) {
  const [selected, setSelected] = useState<string | null>(null), narrow = width < 85, nameWidth = Math.max(8, width - 69)
  return <box flexDirection="column" flexShrink={0} minWidth={0}>
    <text fg={theme.dim} wrapMode="none">{narrow ? "Model / Cost / Calls / Share · in / cached / out" : `${pad("Model", nameWidth)} ${pad("Cost", 11)} ${pad("Calls", 7)} ${pad("Input", 12)} ${pad("Cached", 12)} ${pad("Output", 12)} Share`}</text>
    {!models.length ? <text fg={theme.dim}>No model usage recorded.</text> : models.map((row, i) => {
      const name = str(row.model_id) || "unknown", value = metric === "cost" ? num(row.total_cost) : num(row.tokens_in) + num(row.tokens_out)
      const amount = money(num(row.total_cost)), calls = groupInt(num(row.call_count)), share = pct(value, total)
      const narrowName = pad(name, Math.max(4, width - amount.length - calls.length - share.length - 3))
      return <box key={`${name}/${i}`} flexDirection="column" flexShrink={0} backgroundColor={name === selected ? theme.selBg : undefined} onMouseDown={() => setSelected(name)}>
        <text fg={name === selected ? theme.selFg : theme.fg} wrapMode="none" selectable>{narrow ? `${narrowName} ${amount} ${calls} ${share}` : `${pad(name, nameWidth)} ${pad(money(num(row.total_cost)), 11)} ${pad(groupInt(num(row.call_count)), 7)} ${pad(groupInt(num(row.tokens_in)), 12)} ${pad(groupInt(num(row.tokens_cached)), 12)} ${pad(groupInt(num(row.tokens_out)), 12)} ${pct(value, total)}`}</text>
        {narrow ? <text fg={theme.dim} wrapMode="none">{`in ${groupInt(num(row.tokens_in))}  cache ${groupInt(num(row.tokens_cached))}  out ${groupInt(num(row.tokens_out))}`}</text> : null}
        {name === selected ? <text fg={theme.selFg} wrapMode="word" selectable>{name}</text> : null}
      </box>
    })}
  </box>
}
function stamp(epoch: number, offset: number, dayOnly: boolean): string {
  const d = new Date((epoch + offset) * 1000), day = d.toISOString().slice(0, 10)
  return `${dayOnly ? day : day.slice(5)} ${d.toISOString().slice(11, 16)}`
}
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" })
function pad(value: string, width: number): string {
  const size = Bun.stringWidth(value)
  if (size <= width) return value + " ".repeat(width - size)
  let clipped = ""
  for (const { segment } of graphemes.segment(value)) {
    if (Bun.stringWidth(clipped + segment) > width - 1) break
    clipped += segment
  }
  return clipped + "…" + " ".repeat(Math.max(0, width - Bun.stringWidth(clipped) - 1))
}
