import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { DetailHeader } from './helpers'
import { ConnectorListView } from './connector/ConnectorListView'
import { ProviderForm, type ProviderSavePayload } from './connector/ProviderForm'
import { OAuthConnect } from './connector/OAuthConnect'
import { ModelForm } from './connector/ModelForm'
import { useKoma } from '../../store/koma'
import type { Provider, OAuthProv, OAuthConn, Model } from '../../types/config'

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

// Design reference: Connector = 3 catalogues (Providers / OAuth / Models) as a
// master accordion list; add/edit slides to an inline detail form; delete arms
// inline. Providers/models are the authoritative config slice (pushed by the
// host); OAuth stays a local-only stub (untouched, no backend).
export function ConnectorPanel() {
  const providers = useKoma((s) => s.config.providers)
  const models = useKoma((s) => s.config.models)
  const req = useKoma((s) => s.req)
  const [conns, setConns] = useState<OAuthConn[]>([])
  const [view, setView] = useState<View>({ kind: 'list' })
  const [armed, setArmed] = useState<string | null>(null)

  const back = () => setView({ kind: 'list' })

  const saveProvider = (d: ProviderSavePayload) => {
    // Flat payload matching the daemon's GuiReq::SetProvider. `uuid` is `null`
    // for a new provider (synthetic `d.id` placeholder → daemon mints a uuid).
    // `apiKey` is the typed value only — empty means "leave unchanged" (the
    // form never sees the real stored key; see `ProviderForm`).
    const isNew = view.kind === 'provider' ? view.isNew : false
    req({ r: 'SetProvider', uuid: isNew ? null : d.id, name: d.name, endpoint: d.endpoint, apiKey: d.apiKey })
    back()
  }
  const saveModel = (d: Model) => {
    // Flat payload matching the daemon's GuiReq::SetModel. `d.provider` holds
    // the serving provider's uuid (providerOptions is keyed by uuid); empty
    // route → null.
    const isNew = view.kind === 'model' ? view.isNew : false
    req({
      r: 'SetModel',
      uuid: isNew ? null : d.id,
      name: d.name,
      modelId: d.modelId,
      providerUuid: d.provider,
      route: d.route.trim() ? d.route : null,
      roles: d.roles,
      scope: d.scope,
    })
    back()
  }
  const connect = (provider: OAuthProv) => {
    setConns((l) => [...l, { id: nid('oauth'), provider, account: 'connected' }])
    back()
  }

  // ModelForm's Provider select stores the chosen option's `value` into
  // model.provider, which crosses the wire as `providerUuid` and is resolved by
  // the daemon via `p.uuid == provider` (SetModel + ListModels). So the value
  // MUST be the provider uuid, not its name. OAuth conns stay label-valued
  // (local stub — no daemon uuid to resolve against).
  const providerOptions = [
    ...providers.map((p) => ({ value: p.id, label: p.name })),
    ...conns.map((c) => ({ value: `${c.provider} (oauth)`, label: `${c.provider} (oauth)` })),
  ]

  return (
    <div className="relative h-full overflow-hidden">
      <AnimatePresence initial={false}>
        {view.kind === 'list' ? (
          <motion.div
            key="list"
            className="absolute inset-0"
          >
            <ConnectorListView
              providers={providers}
              conns={conns}
              models={models}
              armed={armed}
              onAddProvider={() => setView({ kind: 'provider', draft: { id: nid('prov'), name: '', endpoint: '', hasKey: false }, isNew: true })}
              onAddModel={() => setView({ kind: 'model', draft: { id: nid('model'), name: '', modelId: '', provider: '', route: '', roles: [], scope: 'global' }, isNew: true })}
              onConnectOAuth={() => setView({ kind: 'oauth' })}
              onEditProvider={(p) => setView({ kind: 'provider', draft: { ...p }, isNew: false })}
              onEditModel={(m) => setView({ kind: 'model', draft: { ...m }, isNew: false })}
              onArm={(id) => setArmed(id)}
              onDisarm={() => setArmed(null)}
              onConfirmProvider={(id) => { req({ r: 'DeleteProvider', uuid: id }); setArmed(null) }}
              onConfirmOAuth={(id) => { setConns((l) => l.filter((x) => x.id !== id)); setArmed(null) }}
              onConfirmModel={(id) => {
                // DeleteModel needs the scope to pick global vs session-local list.
                const scope = models.find((m) => m.id === id)?.scope ?? 'global'
                req({ r: 'DeleteModel', uuid: id, scope })
                setArmed(null)
              }}
            />
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
