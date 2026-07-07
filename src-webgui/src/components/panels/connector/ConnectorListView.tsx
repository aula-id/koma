import { useState } from 'react'
import type { ReactNode } from 'react'
import { AccordionSection } from '../../AccordionSection'
import { Empty, AddBtn, Row } from '../helpers'

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

  return (
    <div className="absolute inset-0 overflow-auto bg-koma-panel">
      <AccordionSection
        title="Providers"
        open={open.providers}
        onToggle={() => setOpen((s) => ({ ...s, providers: !s.providers }))}
        action={<AddBtn label="Add provider" onClick={onAddProvider} />}
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
