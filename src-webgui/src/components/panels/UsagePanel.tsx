import { useEffect } from 'react'
import { BarChart3 } from 'lucide-react'
import { useKoma } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'

// Weekday letters, Date.getDay() order (0 = Sunday).
const DOW = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

// Human-compact token count, mirrors UsageFooter's local `fmtTokens` (not
// exported there — copied here rather than reaching across a component file
// for a one-liner).
function fmtTokens(n: number): string {
  if (n >= 10_000) {
    const k = n / 1000
    return `${k.toFixed(1).replace(/\.0$/, '')}k`
  }
  return `${Math.round(n)}`
}

// Activity-bar "Usage" panel: a read-only LAST-7-DAYS preview off the global
// usage ledger (host-only fetch, see GuiReq UsagePreview). Re-requests fresh
// data every time it mounts — Sidebar conditionally renders panels, so
// switching to this view always re-fires the effect below — and whenever the
// Sidebar header's all/session scope toggle flips or the attached session
// changes. Pinned bottom footer opens the singleton Analytics tab (mirrors
// AgentsPanel's "+ Add agent" footer).
export function UsagePanel() {
  const preview = useKoma((s) => s.usagePreview)
  const scope = useKoma((s) => s.ui.usageScope)
  const sessionId = useKoma((s) => s.session.id)
  const setUsageScope = useKoma((s) => s.setUsageScope)
  const openAnalyticsTab = useKoma((s) => s.openAnalyticsTab)
  const refreshUsagePreview = useKoma((s) => s.refreshUsagePreview)

  // Welcome-screen rule: there's no session to filter "session" scope by, so
  // force back to "all" the instant the session goes away (e.g. detaching back
  // to the start screen while "session" was selected). The re-request effect
  // below picks up the resulting scope change.
  useEffect(() => {
    if (sessionId === null && scope === 'session') setUsageScope('all')
  }, [sessionId, scope, setUsageScope])

  useEffect(() => {
    refreshUsagePreview()
  }, [refreshUsagePreview, scope, sessionId])

  if (!preview) {
    return (
      <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
        <div className="flex h-[22px] flex-none items-center bg-koma-head px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75">
          Last 7 days
        </div>
        <div className="flex min-h-0 flex-1 items-center gap-2 px-3 py-6 text-[12px] text-koma-fg opacity-45">
          <BrailleSpinner size={14} className="opacity-70" />
          Loading usage…
        </div>
        <div className="flex-none border-t border-koma-border p-2">
          <button
            onClick={openAnalyticsTab}
            className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            <BarChart3 size={14} /> See Analytics
          </button>
        </div>
      </div>
    )
  }

  const maxCost = Math.max(...preview.days.map((d) => d.cost), 0)
  const todayEpoch = preview.days[preview.days.length - 1]?.epoch

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="flex h-[22px] flex-none items-center bg-koma-head px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75">
        Last 7 days
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        {/* Totals */}
        <div className="mb-4">
          <div className="text-[20px] font-semibold text-koma-fg">${preview.cost.toFixed(4)}</div>
          <div className="mt-0.5 truncate text-[11px] text-koma-fg opacity-60">
            ↑ {fmtTokens(preview.tokensIn)} · cached {fmtTokens(preview.tokensCached)} · ↓{' '}
            {fmtTokens(preview.tokensOut)} · {preview.calls} calls
          </div>
        </div>

        {/* Bar chart */}
        <div className="mb-4">
          <div className="flex h-12 items-end gap-1.5">
            {preview.days.map((d) => {
              const isToday = d.epoch === todayEpoch
              const barH = maxCost > 0 ? Math.max(2, Math.round((d.cost / maxCost) * 48)) : 0
              const label = DOW[new Date(d.epoch * 1000).getDay()]
              return (
                <div
                  key={d.epoch}
                  className="flex h-12 flex-1 items-end"
                  title={`${label} · $${d.cost.toFixed(4)}`}
                >
                  {d.cost > 0 ? (
                    <div
                      className={`w-full rounded-sm ${isToday ? 'bg-koma-accent' : 'bg-koma-fg opacity-35'}`}
                      style={{ height: barH }}
                    />
                  ) : (
                    <div className="h-[2px] w-full rounded-sm bg-koma-fg opacity-15" />
                  )}
                </div>
              )
            })}
          </div>
          <div className="mt-1 flex gap-1.5">
            {preview.days.map((d) => (
              <div key={d.epoch} className="flex-1 text-center text-[10px] text-koma-fg opacity-40">
                {DOW[new Date(d.epoch * 1000).getDay()][0]}
              </div>
            ))}
          </div>
        </div>

        {/* Top models */}
        {preview.topModels.length > 0 && (
          <div>
            <div className="mb-1 text-[10px] uppercase tracking-wide text-koma-fg opacity-45">
              Top models
            </div>
            {preview.topModels.slice(0, 10).map((m) => (
              <div key={m.modelId} className="flex items-center gap-2 py-1">
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-koma-fg opacity-80">
                  {m.modelId}
                </span>
                <span className="flex-none text-[11px] text-koma-fg opacity-60">
                  ${m.cost.toFixed(4)}
                </span>
              </div>
            ))}
            {preview.topModels.length > 10 && (
              <button
                onClick={openAnalyticsTab}
                className="mt-0.5 text-[11px] text-koma-fg opacity-50 underline-offset-2 transition-opacity hover:opacity-80 hover:underline"
              >
                See more
              </button>
            )}
          </div>
        )}
      </div>
      <div className="flex-none border-t border-koma-border p-2">
        <button
          onClick={openAnalyticsTab}
          className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <BarChart3 size={14} /> See Analytics
        </button>
      </div>
    </div>
  )
}
