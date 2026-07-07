import { useState } from 'react'
import { Field, TextInput, Segmented, Chips, Select, Combobox } from '../form'
import { FormActions } from '../helpers'

type Role = 'main' | 'awareness' | 'safeguard' | 'compactor' | 'planner'
type Scope = 'global' | 'local'
type Model = {
  id: string
  name: string
  modelId: string
  provider: string
  route: string
  roles: Role[]
  scope: Scope
}

const ROLE_OPTIONS: { value: Role; label: string }[] = [
  { value: 'main', label: 'main' },
  { value: 'awareness', label: 'awareness' },
  { value: 'safeguard', label: 'safeguard' },
  { value: 'compactor', label: 'compactor' },
  { value: 'planner', label: 'planner' },
]
// Placeholder pool ONLY to demo the model-id omnisearch interaction (koma
// itself fetches this live per-provider from GET {endpoint}/models).
const DEMO_MODEL_IDS = [
  'anthropic/claude-opus-4',
  'anthropic/claude-sonnet-4',
  'openai/gpt-4.1',
  'openai/gpt-4o',
  'google/gemini-2.5-pro',
  'deepseek/deepseek-v3',
]
// Placeholder pool ONLY to demo the Route field (koma's OpenRouter upstream-
// provider picker: real prices/uptime come from the model's live endpoint
// list, fetched per-model — this is just the interaction/layout demo).
const DEMO_ROUTES: { id: string; label: string; priceIn?: string; priceOut?: string; uptime?: number }[] = [
  { id: 'auto', label: 'Auto (OpenRouter routes)' },
  { id: 'digitalocean', label: 'DigitalOcean', priceIn: '0.10', priceOut: '0.28', uptime: 97 },
  { id: 'xiaomi', label: 'Xiaomi', priceIn: '0.14', priceOut: '0.28', uptime: 100 },
  { id: 'parasail', label: 'Parasail', priceIn: '0.14', priceOut: '0.28' },
  { id: 'venice', label: 'Venice', priceIn: '0.14', priceOut: '0.28' },
  { id: 'deepinfra', label: 'DeepInfra', priceIn: '0.40', priceOut: '2.00', uptime: 98 },
]

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
      className={`flex w-full items-center justify-between rounded px-2 py-1 text-left text-[12px] transition-colors ${
        selected ? 'bg-koma-hover text-koma-fg' : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      <span className="truncate">{label}</span>
      {(priceIn || uptime !== undefined) && (
        <span className="flex-none pl-2 text-[10px] text-koma-fg opacity-50">
          {priceIn && priceOut ? `$${priceIn}/$${priceOut}` : ''}
          {uptime !== undefined ? `  ${uptime}%` : ''}
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

  // Route (OpenRouter-style upstream provider picker) only makes sense once a
  // model id is chosen, and only for API-key providers — not OAuth connections
  // (mirrors koma: Route is OpenRouter-only, gated behind provider + model).
  const showRoute = d.modelId.trim() !== '' && d.provider.trim() !== '' && !d.provider.trim().endsWith('(oauth)')

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
            options={DEMO_MODEL_IDS}
            placeholder="Search or type a model id…"
          />
        </Field>
        {showRoute && (
          <Field label="Route">
            <div className="flex flex-col gap-0.5 rounded border border-koma-border p-1">
              {DEMO_ROUTES.map((r) => (
                <RouteRow
                  key={r.id}
                  label={r.label}
                  priceIn={r.priceIn}
                  priceOut={r.priceOut}
                  uptime={r.uptime}
                  selected={(d.route || 'auto') === r.id}
                  onClick={() => patch({ route: r.id })}
                />
              ))}
            </div>
          </Field>
        )}
        <Field label="Roles">
          <Chips value={d.roles} options={ROLE_OPTIONS} onToggle={toggleRole} />
        </Field>
        <Field label="Scope">
          <Segmented
            value={d.scope}
            onChange={(v) => patch({ scope: v })}
            options={[
              { value: 'global', label: 'Global' },
              { value: 'local', label: 'Local' },
            ]}
          />
        </Field>
      </div>
      <FormActions onCancel={onCancel} onSave={() => onSave(d)} saveDisabled={!d.name.trim()} />
    </>
  )
}
