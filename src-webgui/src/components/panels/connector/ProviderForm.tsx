import { useState } from 'react'
import { Field, TextInput } from '../form'
import { FormActions } from '../helpers'

type Provider = { id: string; name: string; endpoint: string; apiKey: string }

export function ProviderForm({ draft, onSave, onCancel }: { draft: Provider; onSave: (d: Provider) => void; onCancel: () => void }) {
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
