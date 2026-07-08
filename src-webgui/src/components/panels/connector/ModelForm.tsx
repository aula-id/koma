import { useEffect, useState } from 'react'
import { Circle, CircleDot, Loader2 } from 'lucide-react'
import { Field, TextInput, Chips, Select, Combobox } from '../form'
import { FormActions } from '../helpers'
import { useKoma } from '../../../store/koma'
import type { Role, Model, RouteEntry } from '../../../types/config'

const ROLE_OPTIONS: { value: Role; label: string }[] = [
  { value: 'main', label: 'main' },
  { value: 'awareness', label: 'awareness' },
  { value: 'safeguard', label: 'safeguard' },
  { value: 'compactor', label: 'compactor' },
  { value: 'planner', label: 'planner' },
]

// The stored route id for an endpoint — the pinned upstream provider string the
// daemon persists on the model (SetModel.route). Prefer an explicit endpoint id,
// fall back to the provider name.
const routeId = (r: RouteEntry) => r.name ?? r.providerName

// OpenRouter pricing is USD-PER-TOKEN (e.g. "0.0000006"); render it as the more
// familiar USD-per-MILLION-tokens. Returns undefined for missing/non-numeric.
function perMillion(price?: string): string | undefined {
  if (price == null || price === '') return undefined
  const n = Number(price)
  if (!Number.isFinite(n)) return undefined
  const perM = n * 1_000_000
  if (perM === 0) return '0'
  return perM.toFixed(perM < 1 ? 3 : 2).replace(/\.?0+$/, '')
}

function RouteRow({
  label,
  priceIn,
  priceOut,
  uptime,
  selected,
  onClick,
}: {
  label: string
  priceIn?: string
  priceOut?: string
  uptime?: number
  selected: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-checked={selected}
      role="radio"
      className={`flex w-full items-center gap-2 overflow-hidden rounded px-2 py-1 text-left text-[12px] transition-colors ${
        selected
          ? 'bg-koma-accent/15 text-koma-fg opacity-100'
          : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      {/* Radio indicator: filled accent dot when selected, hollow ring otherwise. */}
      {selected ? (
        <CircleDot size={13} className="flex-none text-koma-accent" />
      ) : (
        <Circle size={13} className="flex-none opacity-40" />
      )}
      {/* Provider NAME takes priority: full, no truncation. The price/uptime is a
          compact dim suffix that gives up space (ellipsis) when the row is cramped. */}
      <span className="flex-none whitespace-nowrap">{label}</span>
      {(priceIn || uptime !== undefined) && (
        <span className="min-w-0 flex-1 truncate pl-2 text-right text-[10px] text-koma-fg opacity-50">
          {priceIn && priceOut ? `$${priceIn}/$${priceOut}` : ''}
          {uptime !== undefined ? `  ${Math.round(uptime)}%` : ''}
        </span>
      )}
    </button>
  )
}

export function ModelForm({
  draft,
  providerOptions,
  onSave,
  onCancel,
}: {
  draft: Model
  providerOptions: { value: string; label: string }[]
  onSave: (d: Model) => void
  onCancel: () => void
}) {
  const [d, setD] = useState(draft)
  const patch = (p: Partial<Model>) => setD((x) => ({ ...x, ...p }))
  const toggleRole = (r: Role) =>
    setD((x) => ({ ...x, roles: x.roles.includes(r) ? x.roles.filter((y) => y !== r) : [...x.roles, r] }))

  const modelList = useKoma((s) => s.modelList)
  const routeList = useKoma((s) => s.routeList)
  const req = useKoma((s) => s.req)
  // Live per-provider model-id catalogue fetch, triggered whenever the
  // provider field changes (replaces DEMO_MODEL_IDS).
  useEffect(() => {
    if (d.provider.trim()) req({ r: 'ListModels', provider: d.provider })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.provider])

  // Route (OpenRouter-style upstream provider picker) only makes sense once a
  // model id is chosen, and only for API-key providers — not OAuth connections
  // (mirrors koma: Route is OpenRouter-only, gated behind provider + model).
  const showRoute = d.modelId.trim() !== '' && d.provider.trim() !== '' && !d.provider.trim().endsWith('(oauth)')

  // On-demand ROUTE fetch (replaces DEMO_ROUTES): whenever a provider+model_id
  // pair is set, ask the host for the model's live OpenRouter endpoints
  // (debounced; refires on either change). The reply lands in store.routeList
  // echoing the provider+modelId it was fetched for.
  useEffect(() => {
    if (!showRoute) return
    const t = window.setTimeout(() => {
      req({ r: 'ListRoutes', provider: d.provider, modelId: d.modelId })
    }, 250)
    return () => window.clearTimeout(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.provider, d.modelId, showRoute])

  // Only trust a reply that matches the CURRENT provider+model_id (a stale reply
  // from a prior selection is ignored). `null` = still loading.
  const routes =
    routeList && routeList.provider === d.provider && routeList.modelId === d.modelId
      ? routeList.routes
      : null
  const routesLoading = showRoute && routes === null
  const selectedRoute = d.route || 'auto'

  return (
    <>
      <div className="flex-1 overflow-auto py-1">
        <Field label="Name">
          <TextInput value={d.name} autoFocus placeholder="e.g. Opus main" onChange={(e) => patch({ name: e.target.value })} />
        </Field>
        <Field label="Provider">
          <Select
            value={d.provider}
            options={providerOptions}
            onChange={(v) => patch({ provider: v })}
            placeholder={providerOptions.length ? 'Choose a provider' : 'Add a provider first'}
          />
        </Field>
        <Field label="Model id">
          <Combobox
            value={d.modelId}
            onChange={(v) => patch({ modelId: v })}
            options={modelList}
            placeholder="Search or type a model id…"
          />
        </Field>
        {showRoute && (
          <Field label="Route">
            <div className="flex flex-col gap-0.5 rounded border border-koma-border p-1">
              {/* "Auto" is always the default row — let OpenRouter route. */}
              <RouteRow
                label="Auto"
                selected={selectedRoute === 'auto'}
                onClick={() => patch({ route: '' })}
              />
              {routesLoading ? (
                <div className="flex items-center gap-2 px-2 py-1 text-[11px] text-koma-fg opacity-50">
                  <Loader2 size={12} className="flex-none animate-spin" />
                  Loading routes…
                </div>
              ) : routes && routes.length > 0 ? (
                routes.map((r, i) => {
                  const id = routeId(r)
                  return (
                    <RouteRow
                      key={`${id}-${i}`}
                      label={r.providerName}
                      priceIn={perMillion(r.pricePrompt)}
                      priceOut={perMillion(r.priceCompletion)}
                      uptime={r.uptimeLast30m}
                      selected={selectedRoute === id}
                      onClick={() => patch({ route: id })}
                    />
                  )
                })
              ) : (
                <div className="px-2 py-1 text-[11px] text-koma-fg opacity-40">
                  No upstream routes — using Auto.
                </div>
              )}
            </div>
          </Field>
        )}
        <Field label="Roles">
          <Chips value={d.roles} options={ROLE_OPTIONS} onToggle={toggleRole} />
        </Field>
        <Field label="Scope">
          <Select
            value={d.scope}
            options={[
              { value: 'global', label: 'Global' },
              { value: 'local', label: 'Local' },
            ]}
            onChange={(v) => patch({ scope: v })}
          />
        </Field>
      </div>
      <FormActions onCancel={onCancel} onSave={() => onSave(d)} saveDisabled={!d.name.trim()} />
    </>
  )
}
