import { useEffect, useMemo } from 'react'
import { BarChart3, RefreshCw } from 'lucide-react'
import {
  useKoma,
  type AnalyticsMetric,
  type AnalyticsRange,
  type AnalyticsScope,
  type AnalyticsSeriesPoint,
} from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// Human-compact token count (mirrors UsagePanel).
function fmtTokens(n: number): string {
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
  }
  if (n >= 10_000) {
    return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`
  }
  return `${Math.round(n)}`
}

function fmtCost(n: number): string {
  if (n >= 100) return `$${n.toFixed(2)}`
  if (n >= 1) return `$${n.toFixed(3)}`
  return `$${n.toFixed(4)}`
}

function fmtPct(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`
}

function fmtBucketLabel(epoch: number, range: AnalyticsRange): string {
  const d = new Date(epoch * 1000)
  if (range === 'today') {
    return `${d.getHours().toString().padStart(2, '0')}:00`
  }
  if (range === 'year') {
    return `${d.getMonth() + 1}/${d.getDate()}`
  }
  // 7d / 30d
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

const SCOPES: { id: AnalyticsScope; label: string }[] = [
  { id: 'all', label: 'All sessions' },
  { id: 'session', label: 'Current session' },
]
const RANGES: { id: AnalyticsRange; label: string }[] = [
  { id: 'today', label: 'Today' },
  { id: '7d', label: '7d' },
  { id: '30d', label: '30d' },
  { id: 'year', label: 'Year' },
]
const METRICS: { id: AnalyticsMetric; label: string }[] = [
  { id: 'cost', label: 'Cost' },
  { id: 'tokens', label: 'Tokens' },
]

// Native SVG line+area chart — no chart library, palette tokens only.
function SeriesChart({
  series,
  metric,
  range,
}: {
  series: AnalyticsSeriesPoint[]
  metric: AnalyticsMetric
  range: AnalyticsRange
}) {
  const w = 640
  const h = 160
  const padL = 8
  const padR = 8
  const padT = 12
  const padB = 24
  const innerW = w - padL - padR
  const innerH = h - padT - padB

  // Sparse x labels (avoid crowding). Must be computed before early returns
  // would be added — keep hooks unconditional.
  const labelIdx = useMemo(() => {
    if (series.length <= 8) return series.map((_, i) => i)
    const step = Math.ceil(series.length / 7)
    const out: number[] = []
    for (let i = 0; i < series.length; i += step) out.push(i)
    if (out[out.length - 1] !== series.length - 1) out.push(series.length - 1)
    return out
  }, [series.length])

  const values = series.map((p) => (metric === 'cost' ? p.cost : p.tokens))
  const max = Math.max(...values, 0)
  const points = series.map((p, i) => {
    const x = padL + (series.length <= 1 ? innerW / 2 : (i / (series.length - 1)) * innerW)
    const v = metric === 'cost' ? p.cost : p.tokens
    const y = padT + (max > 0 ? innerH * (1 - v / max) : innerH)
    return { x, y, p, v }
  })
  const line = points.map((pt, i) => `${i === 0 ? 'M' : 'L'}${pt.x.toFixed(1)},${pt.y.toFixed(1)}`).join(' ')
  const area =
    points.length > 0
      ? `${line} L${points[points.length - 1].x.toFixed(1)},${(padT + innerH).toFixed(1)} L${points[0].x.toFixed(1)},${(padT + innerH).toFixed(1)} Z`
      : ''

  return (
    <div className="min-w-0 w-full overflow-hidden">
      <svg
        viewBox={`0 0 ${w} ${h}`}
        preserveAspectRatio="none"
        className="block h-40 w-full"
        role="img"
        aria-label="Usage over time"
      >
        {/* Baseline */}
        <line
          x1={padL}
          y1={padT + innerH}
          x2={padL + innerW}
          y2={padT + innerH}
          className="stroke-koma-border"
          strokeWidth={1}
        />
        {area && <path d={area} className="fill-koma-accent opacity-15" />}
        {line && (
          <path d={line} className="fill-none stroke-koma-accent" strokeWidth={2} strokeLinejoin="round" />
        )}
        {points.map((pt) =>
          pt.v > 0 ? (
            <circle
              key={pt.p.epoch}
              cx={pt.x}
              cy={pt.y}
              r={2.5}
              className="fill-koma-accent"
            >
              <title>
                {fmtBucketLabel(pt.p.epoch, range)} ·{' '}
                {metric === 'cost' ? fmtCost(pt.v) : fmtTokens(pt.v)}
              </title>
            </circle>
          ) : null,
        )}
        {labelIdx.map((i) => (
          <text
            key={series[i].epoch}
            x={points[i].x}
            y={h - 6}
            textAnchor="middle"
            className="fill-koma-fg text-[10px] opacity-40"
          >
            {fmtBucketLabel(series[i].epoch, range)}
          </text>
        ))}
      </svg>
    </div>
  )
}

function SegBtn<T extends string>({
  active,
  label,
  onClick,
  disabled,
}: {
  active: boolean
  label: string
  onClick: () => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`rounded px-2 py-1 text-[11px] transition-colors disabled:cursor-not-allowed disabled:opacity-35 ${
        active
          ? 'bg-koma-accent/15 text-koma-accent'
          : 'text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      {label}
    </button>
  )
}

// Singleton Analytics dashboard tab — palette-token-only, native SVG chart.
// Host-local ledger projection (GuiReq Analytics); filters + stale-reply
// protection live in the `analytics` store slice.
export default function AnalyticsTab() {
  const scope = useKoma((s) => s.analytics.scope)
  const range = useKoma((s) => s.analytics.range)
  const metric = useKoma((s) => s.analytics.metric)
  const loading = useKoma((s) => s.analytics.loading)
  const error = useKoma((s) => s.analytics.error)
  const data = useKoma((s) => s.analytics.data)
  const hasData = useKoma((s) => s.analytics.hasData)
  const sessionId = useKoma((s) => s.session.id)
  const refreshAnalytics = useKoma((s) => s.refreshAnalytics)
  const setAnalyticsScope = useKoma((s) => s.setAnalyticsScope)
  const setAnalyticsRange = useKoma((s) => s.setAnalyticsRange)
  const setAnalyticsMetric = useKoma((s) => s.setAnalyticsMetric)
  const isActiveTab = useKoma((s) => s.ui.activeTabId === 'analytics')

  // Fetch on mount + whenever the attached session changes while this tab is
  // open (session-scope must not keep showing the old session's numbers).
  useEffect(() => {
    refreshAnalytics()
  }, [refreshAnalytics, sessionId])

  // Welcome-screen rule: force "all" when the session goes away.
  useEffect(() => {
    if (sessionId === null && scope === 'session') setAnalyticsScope('all')
  }, [sessionId, scope, setAnalyticsScope])

  // Re-fetch when the tab becomes active again (CSS-hidden, not unmounted).
  useEffect(() => {
    if (isActiveTab) refreshAnalytics()
  }, [isActiveTab, refreshAnalytics])

  const showEmpty = hasData && data && data.calls === 0 && !loading && !error
  const showData = hasData && data && !error

  return (
    <div className="flex h-full min-h-0 flex-col bg-koma-bg text-koma-fg">
      {/* Header */}
      <div className="flex flex-none flex-wrap items-center gap-2 border-b border-koma-border px-4 py-2.5">
        <BarChart3 size={16} className="flex-none text-koma-accent opacity-90" />
        <h1 className="text-[13px] font-semibold text-koma-fg">Analytics</h1>
        <div className="ml-auto flex flex-wrap items-center gap-1">
          <div className="flex items-center gap-0.5 rounded border border-koma-border p-0.5">
            {SCOPES.map((s) => (
              <SegBtn
                key={s.id}
                active={scope === s.id}
                label={s.label}
                disabled={s.id === 'session' && sessionId === null}
                onClick={() => setAnalyticsScope(s.id)}
              />
            ))}
          </div>
          <div className="flex items-center gap-0.5 rounded border border-koma-border p-0.5">
            {RANGES.map((r) => (
              <SegBtn
                key={r.id}
                active={range === r.id}
                label={r.label}
                onClick={() => setAnalyticsRange(r.id)}
              />
            ))}
          </div>
          <div className="flex items-center gap-0.5 rounded border border-koma-border p-0.5">
            {METRICS.map((m) => (
              <SegBtn
                key={m.id}
                active={metric === m.id}
                label={m.label}
                onClick={() => setAnalyticsMetric(m.id)}
              />
            ))}
          </div>
          <button
            type="button"
            onClick={refreshAnalytics}
            title="Refresh"
            className="flex h-7 w-7 items-center justify-center rounded border border-koma-border text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            {loading ? <BrailleSpinner size={13} /> : <RefreshCw size={13} />}
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {loading && !showData && (
          <div className="flex items-center gap-2 py-10 text-[12px] text-koma-fg opacity-45">
            <BrailleSpinner size={14} className="opacity-70" />
            Loading analytics…
          </div>
        )}

        {error && (
          <div className="rounded border border-koma-error/40 bg-koma-error/10 px-3 py-2 text-[12px] text-koma-error">
            {error}
          </div>
        )}

        {showEmpty && (
          <div className="py-10 text-center text-[12px] text-koma-fg opacity-45">
            No usage recorded for this scope and range.
          </div>
        )}

        {showData && data && data.calls > 0 && (
          <div className="flex flex-col gap-5">
            {/* KPI strip */}
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
              <Kpi label="Cost" value={fmtCost(data.cost)} />
              <Kpi label="Calls" value={`${data.calls}`} />
              <Kpi label="Input tokens" value={fmtTokens(data.tokensIn)} />
              <Kpi label="Cached tokens" value={fmtTokens(data.tokensCached)} />
              <Kpi label="Output tokens" value={fmtTokens(data.tokensOut)} />
              <Kpi
                label="Cache rate"
                value={fmtPct(data.cacheRate)}
                hint="cached / (input + cached)"
              />
            </div>

            {/* Time series */}
            <section className="rounded border border-koma-border bg-koma-panel2 p-3">
              <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-50">
                {metric === 'cost' ? 'Cost' : 'Tokens'} over time
              </div>
              <SeriesChart series={data.series} metric={metric} range={range} />
            </section>

            {/* Role breakdown */}
            <section className="rounded border border-koma-border bg-koma-panel2 p-3">
              <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-50">
                Main vs sub-agent
              </div>
              <div className="grid grid-cols-2 gap-3">
                <RoleCard
                  title="Main"
                  cost={data.mainCost}
                  calls={data.mainCalls}
                  totalCost={data.mainCost + data.subCost}
                />
                <RoleCard
                  title="Sub-agents"
                  cost={data.subCost}
                  calls={data.subCalls}
                  totalCost={data.mainCost + data.subCost}
                />
              </div>
            </section>

            {/* Per-model table */}
            <section className="rounded border border-koma-border bg-koma-panel2 p-3">
              <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-50">
                Models
              </div>
              {data.models.length === 0 ? (
                <div className="text-[12px] text-koma-fg opacity-45">No model rows.</div>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[520px] border-collapse text-left text-[12px]">
                    <thead>
                      <tr className="border-b border-koma-border text-[10px] uppercase tracking-wide text-koma-fg opacity-45">
                        <th className="py-1.5 pr-3 font-medium">Model</th>
                        <th className="py-1.5 pr-3 font-medium">Cost</th>
                        <th className="py-1.5 pr-3 font-medium">Calls</th>
                        <th className="py-1.5 pr-3 font-medium">In</th>
                        <th className="py-1.5 pr-3 font-medium">Cached</th>
                        <th className="py-1.5 font-medium">Out</th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.models.map((m) => (
                        <tr key={m.modelId} className="border-b border-koma-border/60">
                          <td className="max-w-[240px] truncate py-1.5 pr-3 font-mono text-[11px] text-koma-fg opacity-90">
                            {m.modelId || '(unknown)'}
                          </td>
                          <td className="py-1.5 pr-3 text-koma-fg opacity-80">{fmtCost(m.cost)}</td>
                          <td className="py-1.5 pr-3 text-koma-fg opacity-70">{m.calls}</td>
                          <td className="py-1.5 pr-3 text-koma-fg opacity-70">{fmtTokens(m.tokensIn)}</td>
                          <td className="py-1.5 pr-3 text-koma-fg opacity-70">
                            {fmtTokens(m.tokensCached)}
                          </td>
                          <td className="py-1.5 text-koma-fg opacity-70">{fmtTokens(m.tokensOut)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            {loading && (
              <div className="flex items-center gap-2 text-[11px] text-koma-fg opacity-40">
                <BrailleSpinner size={12} className="opacity-70" />
                Refreshing…
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function Kpi({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded border border-koma-border bg-koma-panel2 px-3 py-2">
      <div className="text-[10px] uppercase tracking-wide text-koma-fg opacity-45" title={hint}>
        {label}
      </div>
      <div className="mt-0.5 text-[16px] font-semibold text-koma-fg">{value}</div>
    </div>
  )
}

function RoleCard({
  title,
  cost,
  calls,
  totalCost,
}: {
  title: string
  cost: number
  calls: number
  totalCost: number
}) {
  const pct = totalCost > 0 ? Math.round((cost / totalCost) * 100) : 0
  return (
    <div className="rounded border border-koma-border bg-koma-bg px-3 py-2">
      <div className="text-[12px] font-medium text-koma-fg">{title}</div>
      <div className="mt-1 text-[14px] font-semibold text-koma-fg">{fmtCost(cost)}</div>
      <div className="mt-0.5 text-[11px] text-koma-fg opacity-55">
        {calls} calls · {pct}% of cost
      </div>
      <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-koma-panel">
        <div className="h-full bg-koma-accent" style={{ width: `${pct}%` }} />
      </div>
    </div>
  )
}
