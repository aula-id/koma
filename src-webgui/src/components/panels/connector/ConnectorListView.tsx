import { useState } from 'react'
import { AccordionSection } from '../../AccordionSection'
import { Empty, AddBtn, Row, ScopePill } from '../helpers'
import type { Provider, OAuthConn, Model } from '../../../types/config'

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
  onConfirmOAuth: (id: string) => void
  onConfirmModel: (id: string) => void
}

export function ConnectorListView({
  providers, conns, models, armed,
  onAddProvider, onAddModel, onConnectOAuth,
  onEditProvider, onEditModel,
  onArm, onDisarm,
  onConfirmProvider, onConfirmOAuth, onConfirmModel,
}: Props) {
  const [open, setOpen] = useState({ providers: true, oauth: true, models: true })

  // The synthetic koma-free tier is DROPDOWN-ONLY: exclude the auto-provisioned
  // keyless provider (`isKomaFree`) and the free-flagged synthetic model
  // (`m.free`) from the Connector lists so they don't leak as an editable
  // provider / a phantom 2nd "main" model. Only real config providers/models
  // and session_models show here.
  const realProviders = providers.filter((p) => !p.isKomaFree)
  const realModels = models.filter((m) => !m.free)

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <AccordionSection
        title="Providers"
        open={open.providers}
        onToggle={() => setOpen((s) => ({ ...s, providers: !s.providers }))}
        action={<AddBtn label="Add provider" onClick={onAddProvider} />}
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
        action={<AddBtn label="Connect account" onClick={onConnectOAuth} />}
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
            onArm={() => onArm(c.id)}
            onDisarm={onDisarm}
            onConfirm={() => onConfirmOAuth(c.id)}
          />
        ))}
      </AccordionSection>

      <AccordionSection
        title="Models"
        open={open.models}
        onToggle={() => setOpen((s) => ({ ...s, models: !s.models }))}
        action={<AddBtn label="Add model" onClick={onAddModel} />}
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
