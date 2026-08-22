import { useEffect, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { DetailHeader } from './helpers'
import { ConnectorListView } from './connector/ConnectorListView'
import { ProviderForm, type ProviderSavePayload } from './connector/ProviderForm'
import { OAuthConnect } from './connector/OAuthConnect'
import { ModelForm } from './connector/ModelForm'
import { useKoma } from '../../store/koma'
import type { Provider, Model } from '../../types/config'

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
// inline. Providers/models/OAuth conns are all authoritative store slices
// (pushed by the host) — OAuth is real now, wired to the OAuthState push.
export function ConnectorPanel() {
  const providers = useKoma((s) => s.config.providers)
  const models = useKoma((s) => s.config.models)
  const conns = useKoma((s) => s.oauth.conns)
  const req = useKoma((s) => s.req)
  const [view, setView] = useState<View>({ kind: 'list' })
  const [armed, setArmed] = useState<string | null>(null)

  // Fire once when the connector/oauth UI mounts — the OAuth section (list +
  // provider picker) reads straight off the store thereafter; every
  // StartOAuth/SubmitOAuthPaste/CancelOAuth/DeleteOAuthConn also re-pushes a
  // fresh OAuthState, so no polling is needed after this.
  useEffect(() => {
    req({ r: 'GetOAuthState' })
  }, [req])

  const back = () => setView({ kind: 'list' })

  // Both the OAuth detail view's DetailHeader back-arrow AND OAuthConnect's
  // own Cancel button route through this single exit path (passed as
  // `onDone`), so leaving the screen is coherent no matter which control
  // triggers it: mid-flow (anything but 'idle'/'failed') it aborts the
  // running flow server-side — the PKCE browser tab / device poll / paste
  // prompt — via CancelOAuth before dropping back to the list, instead of
  // silently orphaning it. `oauth.phase` is read fresh via getState() (not a
  // subscribed value) since this only needs to be current at click-time.
  const leaveOAuth = () => {
    const phase = useKoma.getState().oauth.phase
    if (phase !== 'idle' && phase !== 'failed') {
      req({ r: 'CancelOAuth' })
    }
    back()
  }

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
  // ModelForm's Provider select stores the chosen option's `value` into
  // model.provider, which crosses the wire as `providerUuid`. The daemon
  // resolves that against EITHER catalogue — a real config provider (uuid ==
  // `Provider.id`) OR an OAuth connection (uuid == `OAuthConn.uuid` —
  // resolve.rs matches `config.oauth_conns` by uuid too, routing the model
  // through that connection's bearer token / chat endpoint). So an OAuth conn
  // IS a fully valid model provider, not just a config.providers entry —
  // offer both. OAuth-backed options are label-suffixed "· OAuth" so the user
  // can tell them apart from a static API-key provider at a glance (the
  // Select component has no option-grouping to lean on instead). The
  // synthetic koma-free provider is dropdown-only (see ConnectorListView) —
  // excluded here too so it can't be hand-picked for a new/edited model (that
  // gateway serves a single fixed model, not an arbitrary modelId).
  const providerOptions = [
    ...providers.filter((p) => !p.isKomaFree).map((p) => ({ value: p.id, label: p.name })),
    ...conns.map((c) => ({ value: c.uuid, label: `${c.name} · OAuth` })),
  ]

  return (
    <div className="relative h-full overflow-hidden" data-tour="connector-panel">
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
              onConfirmOAuth={(uuid) => { req({ r: 'DeleteOAuthConn', uuid }); setArmed(null) }}
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
              onBack={view.kind === 'oauth' ? leaveOAuth : back}
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
            {view.kind === 'oauth' && <OAuthConnect onDone={leaveOAuth} />}
            {view.kind === 'model' && <ModelForm draft={view.draft} providerOptions={providerOptions} onSave={saveModel} onCancel={back} />}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
