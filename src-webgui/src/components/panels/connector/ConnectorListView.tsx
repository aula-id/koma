import { useMemo, useState } from 'react'
import { Check, Trash2, X } from 'lucide-react'
import { AccordionSection } from '../../AccordionSection'
import { Empty, AddBtn, Row, ScopePill } from '../helpers'
import { useKoma, type OAuthConn } from '../../../store/koma'
import type { Provider, Model } from '../../../types/config'

type Props = {
  providers: Provider[]
  conns: OAuthConn[]
  models: Model[]
  armed: string | null
  onAddProvider: () => void
  onAddModel: () => void
  onConnectOAuth: () => void
  onEditProvider: (p: Provider) => void
  onEditModel: (m: Model) => void
  onArm: (id: string) => void
  onDisarm: () => void
  onConfirmProvider: (id: string) => void
  onConfirmOAuth: (uuid: string) => void
  onConfirmModel: (id: string) => void
}

// One OAuth connection row — arm-confirm two-stage like the session-row /
// AgentTab delete pattern (a danger-role-tinted "disconnect?" strip replacing
// the row's trailing action), NOT the generic Row/IconBtn emerald/red
// convention the Providers/Models sections below still use — this dashboard
// deals with real signed-in accounts, so the delete affordance gets the same
// treatment as killing a session or deleting an agent.
function OAuthConnRow({
  conn,
  armed,
  onArm,
  onDisarm,
  onConfirm,
  errorTint,
}: {
  conn: OAuthConn
  armed: boolean
  onArm: () => void
  onDisarm: () => void
  onConfirm: () => void
  errorTint: string
}) {
  if (armed) {
    return (
      <div
        className="flex min-h-[42px] items-center gap-2 px-3 py-1.5 text-[12px] font-medium"
        style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
      >
        <span className="min-w-0 flex-1 truncate">Disconnect {conn.name || conn.provider}?</span>
        <button
          onClick={onConfirm}
          aria-label="Confirm disconnect"
          className="flex flex-none items-center gap-1 rounded px-1.5 font-semibold opacity-90 transition-opacity hover:opacity-100"
          style={{ color: errorTint }}
        >
          <Check size={13} className="flex-none" />
          yes
        </button>
        <button
          onClick={onDisarm}
          aria-label="Cancel disconnect"
          className="flex flex-none items-center gap-1 rounded px-1.5 text-koma-fg opacity-70 transition-opacity hover:opacity-100"
        >
          <X size={13} className="flex-none" />
          no
        </button>
      </div>
    )
  }
  return (
    <div className="group flex min-h-[42px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover">
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] text-koma-fg">{conn.name || conn.email || conn.provider}</div>
        <div className="truncate text-[11px] text-koma-fg opacity-45">
          {conn.email}
          {conn.plan ? ` · ${conn.plan}` : ''}
        </div>
      </div>
      <span className="flex-none rounded bg-koma-head px-1 py-px text-[10px] uppercase tracking-wide text-koma-fg opacity-60">
        {conn.provider}
      </span>
      <button
        onClick={onArm}
        aria-label="Disconnect"
        title="Disconnect"
        style={{ color: errorTint }}
        className="flex h-5 w-5 flex-none items-center justify-center rounded opacity-0 transition group-hover:opacity-60 hover:!opacity-100"
      >
        <Trash2 size={13} />
      </button>
    </div>
  )
}

export function ConnectorListView({
  providers, conns, models, armed,
  onAddProvider, onAddModel, onConnectOAuth,
  onEditProvider, onEditModel,
  onArm, onDisarm,
  onConfirmProvider, onConfirmOAuth, onConfirmModel,
}: Props) {
  const [open, setOpen] = useState({ providers: true, oauth: true, models: true })
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  // Danger/error palette role tint — same lookup ToastContainer/SessionRowActions/
  // AgentTab use (index 9 of the 11-role PaletteInfo.colors array). Never a
  // hardcoded red/orange.
  const errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])

  // The synthetic koma-free tier is DROPDOWN-ONLY: exclude the auto-provisioned
  // keyless provider (`isKomaFree`) and the free-flagged synthetic model
  // (`m.free`) from the Connector lists so they don't leak as an editable
  // provider / a phantom 2nd "main" model. Only real config providers/models
  // and session_models show here.
  const realProviders = providers.filter((p) => !p.isKomaFree)
  const realModels = models.filter((m) => !m.free)

  return (
    <div
      className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel"
      data-tour="connector-list"
    >
      <AccordionSection
        title="Providers"
        open={open.providers}
        onToggle={() => setOpen((s) => ({ ...s, providers: !s.providers }))}
        action={<AddBtn label="Add provider" tourId="connector-add-provider" onClick={onAddProvider} />}
      >
        {realProviders.length === 0 && <Empty>No providers</Empty>}
        {realProviders.map((p) => (
          <Row
            key={p.id}
            title={p.name || '(unnamed)'}
            subtitle={p.endpoint || '—'}
            right={<span className="text-[11px] text-koma-fg opacity-45">{p.hasKey ? '••••' : '—'}</span>}
            confirmLabel={`Remove "${p.name || 'provider'}"?`}
            armed={armed === p.id}
            onEdit={() => onEditProvider(p)}
            onArm={() => onArm(p.id)}
            onDisarm={onDisarm}
            onConfirm={() => onConfirmProvider(p.id)}
          />
        ))}
      </AccordionSection>

      <AccordionSection
        title="OAuth"
        open={open.oauth}
        onToggle={() => setOpen((s) => ({ ...s, oauth: !s.oauth }))}
        action={<AddBtn label="Connect account" tourId="connector-add-oauth" onClick={onConnectOAuth} />}
      >
        {conns.length === 0 && <Empty>No connections</Empty>}
        {conns.map((c) => (
          <OAuthConnRow
            key={c.uuid}
            conn={c}
            armed={armed === c.uuid}
            onArm={() => onArm(c.uuid)}
            onDisarm={onDisarm}
            onConfirm={() => onConfirmOAuth(c.uuid)}
            errorTint={errorTint}
          />
        ))}
      </AccordionSection>

      <AccordionSection
        title="Models"
        open={open.models}
        onToggle={() => setOpen((s) => ({ ...s, models: !s.models }))}
        action={<AddBtn label="Add model" tourId="connector-add-model" onClick={onAddModel} />}
      >
        {realModels.length === 0 && <Empty>No models</Empty>}
        {realModels.map((m) => (
          <Row
            key={m.id}
            leading={<ScopePill scope={m.scope} />}
            title={m.name || '(unnamed)'}
            subtitle={`${m.modelId || '—'}${m.provider ? ' · ' + m.provider : ''}`}
            right={m.roles.length ? <span className="text-[10px] text-koma-fg opacity-55">{m.roles[0]}{m.roles.length > 1 ? ` +${m.roles.length - 1}` : ''}</span> : undefined}
            confirmLabel={`Delete "${m.name || 'model'}"?`}
            armed={armed === m.id}
            onEdit={() => onEditModel(m)}
            onArm={() => onArm(m.id)}
            onDisarm={onDisarm}
            onConfirm={() => onConfirmModel(m.id)}
          />
        ))}
      </AccordionSection>
    </div>
  )
}
