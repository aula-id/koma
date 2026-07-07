import { useState, type ReactNode } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Plus, Pencil, Trash2, Check, X, ChevronLeft } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { Field, TextInput, Segmented, Chips, Select, Combobox } from './form'

type Provider = { id: string; name: string; endpoint: string; apiKey: string }
type OAuthProv = 'OpenAI' | 'Kilo Code' | 'Anthropic'
type OAuthConn = { id: string; provider: OAuthProv; account: string }
type Scope = 'global' | 'local'
type Role = 'main' | 'awareness' | 'safeguard' | 'compactor' | 'planner'
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
const OAUTH_PROVIDERS: OAuthProv[] = ['OpenAI', 'Kilo Code', 'Anthropic']
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
const SLIDE = { type: 'tween', duration: 0.22, ease: 'easeOut' } as const

let seq = 0
const nid = (p: string) => {
  seq += 1
  return `${p}-${seq}`
}

type View =
  | { kind: 'list' }
  | { kind: 'provider'; draft: Provider; isNew: boolean }
  | { kind: 'oauth' }
  | { kind: 'model'; draft: Model; isNew: boolean }

function IconBtn({
  children,
  label,
  onClick,
  tone,
  faded,
}: {
  children: ReactNode
  label: string
  onClick?: () => void
  tone?: 'emerald' | 'red'
  faded?: boolean
}) {
  const hover = tone === 'emerald' ? 'hover:!text-emerald-500' : tone === 'red' ? 'hover:!text-red-500' : ''
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className={`flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg transition ${
        faded ? 'opacity-0 group-hover:opacity-60' : 'opacity-70'
      } hover:!opacity-100 ${hover}`}
    >
      {children}
    </button>
  )
}

type RowProps = {
  leading?: ReactNode
  title: string
  subtitle?: string
  right?: ReactNode
  confirmLabel: string
  armed: boolean
  onEdit?: () => void
  onArm: () => void
  onDisarm: () => void
  onConfirm: () => void
}

function Row({ leading, title, subtitle, right, confirmLabel, armed, onEdit, onArm, onDisarm, onConfirm }: RowProps) {
  if (armed) {
    return (
      <div className="flex min-h-[42px] items-center gap-2 px-3 py-1.5">
        <span className="flex-1 truncate text-[12px] text-koma-fg">{confirmLabel}</span>
        <IconBtn label="Confirm" tone="emerald" onClick={onConfirm}>
          <Check size={14} />
        </IconBtn>
        <IconBtn label="Cancel" tone="red" onClick={onDisarm}>
          <X size={14} />
        </IconBtn>
      </div>
    )
  }
  return (
    <div className="group flex min-h-[42px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover">
      {leading}
      <button onClick={onEdit} disabled={!onEdit} className="min-w-0 flex-1 text-left disabled:cursor-default">
        <div className="truncate text-[13px] text-koma-fg">{title}</div>
        {subtitle && <div className="truncate text-[11px] text-koma-fg opacity-45">{subtitle}</div>}
      </button>
      {right && <div className="flex-none">{right}</div>}
      {onEdit && (
        <IconBtn label="Edit" faded onClick={onEdit}>
          <Pencil size={13} />
        </IconBtn>
      )}
      <IconBtn label="Delete" tone="red" faded onClick={onArm}>
        <Trash2 size={13} />
      </IconBtn>
    </div>
  )
}

function Empty({ children }: { children: string }) {
  return <div className="px-5 py-1.5 text-[12px] text-koma-fg opacity-35">{children}</div>
}

function AddBtn({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
    >
      <Plus size={14} />
    </button>
  )
}

function DetailHeader({ onBack, title }: { onBack: () => void; title: string }) {
  return (
    <div className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2">
      <button
        onClick={onBack}
        aria-label="Back"
        className="flex h-6 w-6 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronLeft size={16} />
      </button>
      <span className="text-[12px] font-semibold text-koma-fg">{title}</span>
    </div>
  )
}

function FormActions({ onCancel, onSave, saveDisabled }: { onCancel: () => void; onSave: () => void; saveDisabled?: boolean }) {
  return (
    <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2">
      <button
        onClick={onCancel}
        className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        Cancel
      </button>
      <button
        onClick={onSave}
        disabled={saveDisabled}
        className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
      >
        Save
      </button>
    </div>
  )
}

function ProviderForm({ draft, onSave, onCancel }: { draft: Provider; onSave: (d: Provider) => void; onCancel: () => void }) {
  const [d, setD] = useState(draft)
  const patch = (p: Partial<Provider>) => setD((x) => ({ ...x, ...p }))
  return (
    <>
      <div className="flex-1 overflow-auto py-1">
        <Field label="Name">
          <TextInput value={d.name} autoFocus placeholder="e.g. OpenRouter" onChange={(e) => patch({ name: e.target.value })} />
        </Field>
        <Field label="Endpoint (base URL)">
          <TextInput value={d.endpoint} placeholder="https://…/v1" onChange={(e) => patch({ endpoint: e.target.value })} />
        </Field>
        <Field label="API key">
          <TextInput value={d.apiKey} type="password" placeholder="sk-…" onChange={(e) => patch({ apiKey: e.target.value })} />
        </Field>
      </div>
      <FormActions onCancel={onCancel} onSave={() => onSave(d)} saveDisabled={!d.name.trim()} />
    </>
  )
}

function OAuthConnect({ onPick, onCancel }: { onPick: (p: OAuthProv) => void; onCancel: () => void }) {
  return (
    <>
      <div className="flex-1 overflow-auto p-3">
        <div className="mb-2 text-[11px] text-koma-fg opacity-50">Choose a provider to connect</div>
        <div className="flex flex-col gap-1.5">
          {OAUTH_PROVIDERS.map((p) => (
            <button
              key={p}
              onClick={() => onPick(p)}
              className="flex items-center justify-between rounded border border-koma-border px-3 py-2 text-[13px] text-koma-fg transition-colors hover:bg-koma-hover"
            >
              <span>{p}</span>
              <span className="text-[10px] uppercase tracking-wide text-koma-fg opacity-40">
                {p === 'Kilo Code' ? 'device code' : 'browser'}
              </span>
            </button>
          ))}
        </div>
        <div className="mt-3 text-[11px] text-koma-fg opacity-35">Opens the provider’s sign-in (stub — no backend yet).</div>
      </div>
      <div className="flex flex-none items-center justify-end border-t border-koma-border px-3 py-2">
        <button
          onClick={onCancel}
          className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
      </div>
    </>
  )
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

function ModelForm({
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

// Design reference: Connector = 3 catalogues (Providers / OAuth / Models) as a
// master accordion list; add/edit slides to an inline detail form; delete arms
// inline. Local state, starts empty, no popups, no backend.
export function ConnectorPanel() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [conns, setConns] = useState<OAuthConn[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [view, setView] = useState<View>({ kind: 'list' })
  const [armed, setArmed] = useState<string | null>(null)

  const back = () => setView({ kind: 'list' })

  const saveProvider = (d: Provider) => {
    setProviders((l) => (view.kind === 'provider' && view.isNew ? [...l, d] : l.map((x) => (x.id === d.id ? d : x))))
    back()
  }
  const saveModel = (d: Model) => {
    setModels((l) => (view.kind === 'model' && view.isNew ? [...l, d] : l.map((x) => (x.id === d.id ? d : x))))
    back()
  }
  const connect = (provider: OAuthProv) => {
    setConns((l) => [...l, { id: nid('oauth'), provider, account: 'connected' }])
    back()
  }

  const providerOptions = [
    ...providers.map((p) => ({ value: p.name, label: p.name })),
    ...conns.map((c) => ({ value: `${c.provider} (oauth)`, label: `${c.provider} (oauth)` })),
  ]

  return (
    <div className="relative h-full overflow-hidden">
      <AnimatePresence initial={false}>
        {view.kind === 'list' ? (
          <motion.div
            key="list"
            initial={{ x: '-25%', opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={{ x: '-25%', opacity: 0 }}
            transition={SLIDE}
            className="absolute inset-0 overflow-auto bg-koma-panel"
          >
            <AccordionSection
              title="Providers"
              action={<AddBtn label="Add provider" onClick={() => setView({ kind: 'provider', draft: { id: nid('prov'), name: '', endpoint: '', apiKey: '' }, isNew: true })} />}
            >
              {providers.length === 0 && <Empty>No providers</Empty>}
              {providers.map((p) => (
                <Row
                  key={p.id}
                  title={p.name || '(unnamed)'}
                  subtitle={p.endpoint || '—'}
                  right={<span className="text-[11px] text-koma-fg opacity-45">{p.apiKey ? '••••' : '—'}</span>}
                  confirmLabel={`Remove "${p.name || 'provider'}"?`}
                  armed={armed === p.id}
                  onEdit={() => setView({ kind: 'provider', draft: { ...p }, isNew: false })}
                  onArm={() => setArmed(p.id)}
                  onDisarm={() => setArmed(null)}
                  onConfirm={() => {
                    setProviders((l) => l.filter((x) => x.id !== p.id))
                    setArmed(null)
                  }}
                />
              ))}
            </AccordionSection>

            <AccordionSection
              title="OAuth"
              action={<AddBtn label="Connect account" onClick={() => setView({ kind: 'oauth' })} />}
            >
              {conns.length === 0 && <Empty>No connections</Empty>}
              {conns.map((c) => (
                <Row
                  key={c.id}
                  title={c.provider}
                  subtitle={c.account}
                  right={<span className="text-[10px] uppercase tracking-wide text-emerald-500">active</span>}
                  confirmLabel={`Disconnect ${c.provider}?`}
                  armed={armed === c.id}
                  onArm={() => setArmed(c.id)}
                  onDisarm={() => setArmed(null)}
                  onConfirm={() => {
                    setConns((l) => l.filter((x) => x.id !== c.id))
                    setArmed(null)
                  }}
                />
              ))}
            </AccordionSection>

            <AccordionSection
              title="Models"
              action={<AddBtn label="Add model" onClick={() => setView({ kind: 'model', draft: { id: nid('model'), name: '', modelId: '', provider: '', route: '', roles: [], scope: 'global' }, isNew: true })} />}
            >
              {models.length === 0 && <Empty>No models</Empty>}
              {models.map((m) => (
                <Row
                  key={m.id}
                  leading={<span title={m.scope} className="w-3 flex-none text-center text-[11px] text-koma-fg opacity-60">{m.scope === 'global' ? '★' : ''}</span>}
                  title={m.name || '(unnamed)'}
                  subtitle={`${m.modelId || '—'}${m.provider ? ' · ' + m.provider : ''}`}
                  right={m.roles.length ? <span className="text-[10px] text-koma-fg opacity-55">{m.roles[0]}{m.roles.length > 1 ? ` +${m.roles.length - 1}` : ''}</span> : undefined}
                  confirmLabel={`Delete "${m.name || 'model'}"?`}
                  armed={armed === m.id}
                  onEdit={() => setView({ kind: 'model', draft: { ...m }, isNew: false })}
                  onArm={() => setArmed(m.id)}
                  onDisarm={() => setArmed(null)}
                  onConfirm={() => {
                    setModels((l) => l.filter((x) => x.id !== m.id))
                    setArmed(null)
                  }}
                />
              ))}
            </AccordionSection>
          </motion.div>
        ) : (
          <motion.div
            key="detail"
            initial={{ x: '100%' }}
            animate={{ x: 0 }}
            exit={{ x: '100%' }}
            transition={SLIDE}
            className="absolute inset-0 flex flex-col bg-koma-panel"
          >
            <DetailHeader
              onBack={back}
              title={
                view.kind === 'provider'
                  ? view.isNew
                    ? 'Add provider'
                    : 'Edit provider'
                  : view.kind === 'oauth'
                    ? 'Connect account'
                    : view.isNew
                      ? 'Add model'
                      : 'Edit model'
              }
            />
            {view.kind === 'provider' && <ProviderForm draft={view.draft} onSave={saveProvider} onCancel={back} />}
            {view.kind === 'oauth' && <OAuthConnect onPick={connect} onCancel={back} />}
            {view.kind === 'model' && <ModelForm draft={view.draft} providerOptions={providerOptions} onSave={saveModel} onCancel={back} />}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
