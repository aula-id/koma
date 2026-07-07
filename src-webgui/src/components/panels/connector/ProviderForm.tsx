import { useState } from 'react'
import { Field, TextInput } from '../form'
import { FormActions } from '../helpers'
import type { Provider } from '../../../types/config'

// The daemon never sends the plaintext key back (see `Provider.hasKey`), so
// the form can't prefill it. The save payload carries only the TYPED value —
// empty means "leave unchanged" (daemon-side blank-keeps-existing).
export type ProviderSavePayload = { id: string; name: string; endpoint: string; apiKey: string }

export function ProviderForm({ draft, onSave, onCancel }: { draft: Provider; onSave: (d: ProviderSavePayload) => void; onCancel: () => void }) {
  const [d, setD] = useState(draft)
  // Always starts empty — the real key is never available to prefill.
  const [apiKey, setApiKey] = useState('')
  const patch = (p: Partial<Provider>) => setD((x) => ({ ...x, ...p }))
  const keyPlaceholder = d.hasKey ? '•••••••• (unchanged — leave blank to keep)' : 'sk-…'
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
          <TextInput value={apiKey} type="password" placeholder={keyPlaceholder} onChange={(e) => setApiKey(e.target.value)} />
        </Field>
      </div>
      <FormActions
        onCancel={onCancel}
        onSave={() => onSave({ id: d.id, name: d.name, endpoint: d.endpoint, apiKey })}
        saveDisabled={!d.name.trim()}
      />
    </>
  )
}
