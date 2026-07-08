import { useState } from 'react'
import { ChevronRight, Server, SlidersHorizontal } from 'lucide-react'
import { Field, TextInput } from '../form'
import { FormActions } from '../helpers'
import type { Provider } from '../../../types/config'

// The daemon never sends the plaintext key back (see `Provider.hasKey`), so
// the form can't prefill it. The save payload carries only the TYPED value —
// empty means "leave unchanged" (daemon-side blank-keeps-existing).
export type ProviderSavePayload = { id: string; name: string; endpoint: string; apiKey: string }

// Predefined marketplaces — a name + verified OpenAI-compatible base URL that
// pre-fills the form so the user only pastes a key. Base URLs are WEB-VERIFIED
// canonical endpoints (note the non-obvious ones: Groq `/openai/v1`, Fireworks
// `/inference/v1`, DeepInfra `/v1/openai`, Mimo token-plan host). "Custom" is
// handled separately (blank manual form).
const PREDEFINED: { name: string; endpoint: string }[] = [
  { name: 'OpenRouter', endpoint: 'https://openrouter.ai/api/v1' },
  { name: 'DeepSeek', endpoint: 'https://api.deepseek.com' },
  { name: 'Mimo (token plan)', endpoint: 'https://token-plan-sgp.xiaomimimo.com/v1' },
  { name: 'OpenAI', endpoint: 'https://api.openai.com/v1' },
  { name: 'Groq', endpoint: 'https://api.groq.com/openai/v1' },
  { name: 'Together', endpoint: 'https://api.together.xyz/v1' },
  { name: 'Fireworks', endpoint: 'https://api.fireworks.ai/inference/v1' },
  { name: 'Mistral', endpoint: 'https://api.mistral.ai/v1' },
  { name: 'DeepInfra', endpoint: 'https://api.deepinfra.com/v1/openai' },
]

export function ProviderForm({ draft, onSave, onCancel }: { draft: Provider; onSave: (d: ProviderSavePayload) => void; onCancel: () => void }) {
  const [d, setD] = useState(draft)
  // Always starts empty — the real key is never available to prefill.
  const [apiKey, setApiKey] = useState('')
  // A brand-new provider (empty name + endpoint) leads with the marketplace
  // picker; editing an existing one jumps straight to the manual form.
  const isNewDraft = draft.name.trim() === '' && draft.endpoint.trim() === ''
  const [step, setStep] = useState<'pick' | 'form'>(isNewDraft ? 'pick' : 'form')
  const patch = (p: Partial<Provider>) => setD((x) => ({ ...x, ...p }))
  const keyPlaceholder = d.hasKey ? '•••••••• (unchanged — leave blank to keep)' : 'sk-…'

  const pickPreset = (name: string, endpoint: string) => {
    patch({ name, endpoint })
    setStep('form')
  }

  if (step === 'pick') {
    return (
      <div className="flex-1 overflow-auto py-1">
        <div className="px-3 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
          Choose a provider
        </div>
        <div className="flex flex-col gap-0.5 px-2">
          {PREDEFINED.map((p) => (
            <button
              key={p.name}
              type="button"
              onClick={() => pickPreset(p.name, p.endpoint)}
              className="flex items-center gap-2 rounded px-2 py-1.5 text-left transition-colors hover:bg-koma-hover"
            >
              <Server size={14} className="flex-none text-koma-accent" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="text-[12.5px] text-koma-fg">{p.name}</span>
                <span className="truncate text-[10.5px] text-koma-fg opacity-40">{p.endpoint}</span>
              </span>
              <ChevronRight size={13} className="flex-none text-koma-fg opacity-30" />
            </button>
          ))}
          <button
            type="button"
            onClick={() => setStep('form')}
            className="mt-1 flex items-center gap-2 rounded border border-dashed border-koma-border px-2 py-1.5 text-left transition-colors hover:bg-koma-hover"
          >
            <SlidersHorizontal size={14} className="flex-none text-koma-fg opacity-60" />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="text-[12.5px] text-koma-fg">Custom</span>
              <span className="text-[10.5px] text-koma-fg opacity-40">Enter name + endpoint manually</span>
            </span>
            <ChevronRight size={13} className="flex-none text-koma-fg opacity-30" />
          </button>
        </div>
      </div>
    )
  }

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
