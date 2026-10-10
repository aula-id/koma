import { pagePoint } from '../lib/uiScale'
import type { RefObject } from 'react'
import { createPortal } from 'react-dom'
import { useKoma } from '../store/koma'

// Human-compact token count: >=10_000 collapses to "12.4k" (one decimal,
// trailing ".0" trimmed). Shared with the footer readout.
export function fmtTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  if (n >= 10_000) {
    const k = n / 1000
    return `${k.toFixed(1).replace(/\.0$/, '')}k`
  }
  return `${Math.round(n)}`
}

export function fmtBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 MB'
  const mb = n / (1024 * 1024)
  if (mb >= 1024) {
    const gb = mb / 1024
    return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1)} GB`
  }
  if (mb >= 100) return `${Math.round(mb)} MB`
  return `${mb.toFixed(1)} MB`
}

function fmtBytesShort(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  const mb = n / (1024 * 1024)
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)}G`
  if (mb >= 10) return `${Math.round(mb)}M`
  return `${mb.toFixed(1)}M`
}

const WINDOW = 'var(--koma-info, #50c8ff)'
const AGENT = 'var(--koma-accent, #39ff14)'
const SERVICES = 'var(--koma-warn, #ffb43c)'

function UsageRing({ parts, center }: { parts: { frac: number; color: string }[]; center: string }) {
  const r = 26
  const c = 2 * Math.PI * r
  let offset = 0
  return (
    <svg width="72" height="72" viewBox="0 0 72 72" className="flex-none text-koma-fg" aria-hidden>
      <circle cx="36" cy="36" r={r} fill="none" stroke="currentColor" strokeWidth="7" className="text-koma-border" />
      {parts.map((part, i) => {
        const frac = Math.max(0, Math.min(1, part.frac))
        const dash = frac * c
        const node = (
          <circle
            key={i}
            cx="36"
            cy="36"
            r={r}
            fill="none"
            stroke={part.color}
            strokeWidth="7"
            strokeDasharray={`${dash} ${Math.max(0, c - dash)}`}
            strokeDashoffset={-offset}
            transform="rotate(-90 36 36)"
          />
        )
        offset += dash
        return node
      })}
      <text x="36" y="37" textAnchor="middle" dominantBaseline="central" fill="currentColor" fontSize="12" fontWeight={600}>
        {center}
      </text>
    </svg>
  )
}

function LegendRow({ color, label, value }: { color?: string; label: string; value: string }) {
  return (
    <div className="flex items-center gap-2 text-[11px] leading-4">
      <span className="h-2 w-2 flex-none rounded-[2px]" style={{ background: color ?? 'transparent' }} />
      <span className="text-koma-dim">{label}</span>
      <span className="ml-auto tabular-nums text-koma-fg">{value}</span>
    </div>
  )
}

// Same order and wording as `role_usage_lines` in procmem.rs.
const USAGE_ROLES = [
  ['main', 'Main'],
  ['awareness', 'Awareness'],
  ['safeguard', 'Safeguard'],
  ['compactor', 'Compactor'],
  ['planner', 'Planner'],
] as const

type RoleRoute = {
  role: string
  effective_model: string | null
  configured_model: string | null
  provider_uuid: string | null
}

// `Provider · model id`. The model id alone when the provider has no name.
// An em dash when the role has no model. Inherited roles still show the model
// they will call.
export function roleUsageValue(route: RoleRoute | undefined, providerName: (uuid: string) => string | undefined): string {
  const model = route?.effective_model?.trim() || route?.configured_model?.trim() || ''
  if (!model) return '—'
  const name = route?.provider_uuid ? providerName(route.provider_uuid)?.trim() : ''
  return name ? `${name} · ${model}` : model
}

// Quick usage card, opened upward from the footer. Same numbers the macOS
// menu-bar extra draws: this window (plus its WebKit helpers), the session
// agent, helper daemons, and the live context / token counters.
export function UsageDash({ rect, menuRef }: { rect: DOMRect; menuRef: RefObject<HTMLDivElement | null> }) {
  const working = useKoma((s) => s.session.working)
  const tokensIn = useKoma((s) => s.session.tokensIn)
  const tokensCached = useKoma((s) => s.session.tokensCached)
  const tokensOut = useKoma((s) => s.session.tokensOut)
  const cost = useKoma((s) => s.session.cost)
  const contextWindow = useKoma((s) => s.session.contextWindow)
  const memWindow = useKoma((s) => s.session.memWindow)
  const memAgent = useKoma((s) => s.session.memAgent)
  const memServices = useKoma((s) => s.session.memServices)
  const memSystem = useKoma((s) => s.session.memSystem)
  const modelRoutes = useKoma((s) => s.session.modelRoutes)
  const providers = useKoma((s) => s.config.providers)
  const oauthConns = useKoma((s) => s.oauth.conns)
  const memTotal = memWindow + memAgent + memServices
  const memKnown = memSystem > 0 || memTotal > 0
  const memText = (n: number) => (memKnown ? fmtBytes(n) : '—')
  const pct = contextWindow > 0 ? Math.round((tokensIn / contextWindow) * 100) : null
  const memParts =
    memTotal > 0
      ? [
          { frac: memWindow / memTotal, color: WINDOW },
          { frac: memAgent / memTotal, color: AGENT },
          { frac: memServices / memTotal, color: SERVICES },
        ].filter((part) => part.frac > 0)
      : []
  const tokenParts =
    contextWindow > 0
      ? [{ frac: Math.min(1, tokensIn / contextWindow), color: AGENT }]
      : []
  const providerName = (uuid: string) => {
    const fromProvider = providers.find((provider) => provider.id === uuid)?.name
    if (fromProvider?.trim()) return fromProvider
    return oauthConns.find((conn) => conn.uuid === uuid)?.name
  }
  const roleLines = USAGE_ROLES.map(([key, label]) => ({
    label,
    value: roleUsageValue(
      modelRoutes.find((route) => route.role === key),
      providerName,
    ),
  }))

  return createPortal(
    <div
      ref={menuRef}
      role="dialog"
      aria-label="Koma usage"
      style={{
        position: 'fixed',
        right: Math.max(8, pagePoint(window.innerWidth) - rect.right),
        bottom: pagePoint(window.innerHeight) - rect.top + 6,
        width: 300,
        maxHeight: Math.max(160, rect.top - 12),
        zIndex: 80,
      }}
      className="overflow-x-hidden overflow-y-auto rounded-lg border border-koma-border bg-koma-bg text-koma-fg shadow-xl"
    >
      <div className="flex items-center gap-2 border-b border-koma-border px-3 py-2">
        <span className={`h-1.5 w-1.5 rounded-full ${working ? 'bg-koma-accent' : 'bg-koma-dim/40'}`} />
        <span className="text-[13px] font-semibold">Koma</span>
      </div>
      <div className="px-3 pt-2 pb-2">
        <div className="mb-1 text-[12px] font-semibold">Memory</div>
        <div className="flex items-center gap-3">
          <UsageRing parts={memParts} center={memKnown ? fmtBytesShort(memTotal) : '—'} />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <LegendRow color={WINDOW} label="Window" value={memText(memWindow)} />
            <LegendRow color={AGENT} label="Agent" value={memText(memAgent)} />
            {(!memKnown || memServices > 0) && (
              <LegendRow color={SERVICES} label="Services" value={memText(memServices)} />
            )}
            <LegendRow label="Total" value={memText(memTotal)} />
            {memSystem > 0 && <div className="pl-4 text-[10px] text-koma-dim">of {fmtBytes(memSystem)}</div>}
          </div>
        </div>
      </div>
      <div className="border-t border-koma-border px-3 pt-2 pb-3">
        <div className="mb-1 text-[12px] font-semibold">Tokens</div>
        <div className="flex items-center gap-3">
          <UsageRing parts={tokenParts} center={pct == null ? '—' : `${pct}%`} />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <LegendRow color={AGENT} label="Context" value={pct == null ? '—' : `${pct}%`} />
            <LegendRow label="In" value={fmtTokens(tokensIn)} />
            <LegendRow label="Cached" value={fmtTokens(tokensCached)} />
            <LegendRow label="Out" value={fmtTokens(tokensOut)} />
            <LegendRow label="Cost" value={`$${cost.toFixed(4)}`} />
          </div>
        </div>
      </div>
      <div className="border-t border-koma-border px-3 pt-2 pb-3">
        <div className="mb-1 text-[12px] font-semibold">Roles</div>
        <div className="flex flex-col gap-1">
          {roleLines.map((line) => (
            <div key={line.label} className="flex items-baseline gap-2 text-[11px] leading-4" title={line.value}>
              <span className="w-16 flex-none text-koma-dim">{line.label}</span>
              <span className="min-w-0 flex-1 truncate text-right text-koma-fg">{line.value}</span>
            </div>
          ))}
        </div>
      </div>
    </div>,
    document.body,
  )
}
