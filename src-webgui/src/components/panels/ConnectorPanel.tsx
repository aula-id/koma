import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { DetailHeader } from './helpers'
import { ConnectorListView } from './connector/ConnectorListView'
import { ProviderForm } from './connector/ProviderForm'
import { OAuthConnect } from './connector/OAuthConnect'
import { ModelForm } from './connector/ModelForm'

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
          >
            <ConnectorListView
              providers={providers}
              conns={conns}
              models={models}
              armed={armed}
              onAddProvider={() => setView({ kind: 'provider', draft: { id: nid('prov'), name: '', endpoint: '', apiKey: '' }, isNew: true })}
              onAddModel={() => setView({ kind: 'model', draft: { id: nid('model'), name: '', modelId: '', provider: '', route: '', roles: [], scope: 'global' }, isNew: true })}
              onConnectOAuth={() => setView({ kind: 'oauth' })}
              onEditProvider={(p) => setView({ kind: 'provider', draft: { ...p }, isNew: false })}
              onEditModel={(m) => setView({ kind: 'model', draft: { ...m }, isNew: false })}
              onArm={(id) => setArmed(id)}
              onDisarm={() => setArmed(null)}
              onConfirmProvider={(id) => { setProviders((l) => l.filter((x) => x.id !== id)); setArmed(null) }}
              onConfirmOAuth={(id) => { setConns((l) => l.filter((x) => x.id !== id)); setArmed(null) }}
              onConfirmModel={(id) => { setModels((l) => l.filter((x) => x.id !== id)); setArmed(null) }}
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
