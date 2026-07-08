import { useMemo, useState } from 'react'
import { ArrowLeft, ArrowRight, Check, Lock, Palette, Plug } from 'lucide-react'
import { useKoma } from '../store/koma'
import { ProviderForm, type ProviderSavePayload } from './panels/connector/ProviderForm'
import { ModelForm } from './panels/connector/ModelForm'
import type { Model } from '../types/config'

let seq = 0
const nid = (p: string) => {
  seq += 1
  return `${p}-${seq}`
}

// Pretty-cases a registry theme key ("github dark" -> "Github Dark") for display.
function titleCase(s: string) {
  return s.replace(/\b\w/g, (c) => c.toUpperCase())
}

function StepDot({ n, active, done, label }: { n: number; active: boolean; done: boolean; label: string }) {
  return (
    <div className="flex items-center gap-2">
      <span
        className={`flex h-6 w-6 flex-none items-center justify-center rounded-full border text-[11px] font-semibold ${
          active
            ? 'border-koma-accent bg-koma-accent/15 text-koma-accent'
            : done
              ? 'border-koma-accent/50 text-koma-accent'
              : 'border-koma-border text-koma-fg opacity-40'
        }`}
      >
        {done ? <Check size={12} /> : n}
      </span>
      <span className={`text-[12px] ${active ? 'text-koma-fg' : 'text-koma-fg opacity-45'}`}>{label}</span>
    </div>
  )
}

// FIRST-RUN full-screen onboarding — rendered ahead of the start screen while
// the config has no usable provider/model (host first-run flag, else inferred
// from an empty config). Two steps, nicer than the CLI wizard:
//   1. Theme  — pick a palette from the host registry; selecting it live-repaints
//               the whole GUI (GuiReq SetTheme -> Config palette re-push).
//   2. Connection — reuse the Connector ProviderForm + ModelForm (SetProvider /
//               SetModel). OAuth is stubbed/greyed. Once a provider + a Main
//               model exist the gate (IndexPage) drops to the start screen.
export function Onboarding() {
  const themes = useKoma((s) => s.config.themes)
  const activeTheme = useKoma((s) => s.config.theme)
  const providers = useKoma((s) => s.config.providers)
  const req = useKoma((s) => s.req)

  const [step, setStep] = useState<1 | 2>(1)
  // Local highlight source of truth (works even if the host doesn't echo the
  // theme name back on Config). Selecting also fires SetTheme for the live paint.
  const [picked, setPicked] = useState(activeTheme)

  const selectTheme = (name: string) => {
    setPicked(name)
    req({ r: 'SetTheme', name })
  }

  // Connection step drives off the authoritative config: no provider yet -> add
  // one; once it lands (host re-pushes providers[]) -> add the model.
  const hasProvider = providers.length > 0
  const providerOptions = useMemo(
    () => providers.map((p) => ({ value: p.id, label: p.name })),
    [providers],
  )

  const saveProvider = (d: ProviderSavePayload) => {
    req({ r: 'SetProvider', uuid: null, name: d.name, endpoint: d.endpoint, apiKey: d.apiKey })
    // Stay on step 2 — the provider push flips hasProvider and swaps in ModelForm.
  }
  const saveModel = (d: Model) => {
    req({
      r: 'SetModel',
      uuid: null,
      name: d.name,
      modelId: d.modelId,
      providerUuid: d.provider,
      route: d.route.trim() ? d.route : null,
      roles: d.roles,
      scope: d.scope,
    })
    // A Main model completes onboarding — the IndexPage gate unmounts this.
  }

  // Fresh drafts, stable across re-renders of the same sub-step. The model draft
  // pre-selects the (first) provider + seeds the Main role so completing it
  // satisfies the "usable Main" gate immediately.
  const providerDraft = useMemo(() => ({ id: nid('prov'), name: '', endpoint: '', hasKey: false }), [])
  const modelDraft = useMemo<Model>(
    () => ({
      id: nid('model'),
      name: '',
      modelId: '',
      provider: providers[0]?.id ?? '',
      route: '',
      roles: ['main'],
      scope: 'global',
    }),
    // Re-seed the provider once one exists (keyed on the transition to the model form).
    [hasProvider], // eslint-disable-line react-hooks/exhaustive-deps
  )

  return (
    <div className="h-full w-full overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-[560px] flex-col px-6 py-10">
        {/* Header + stepper */}
        <div className="mb-1 flex items-baseline gap-2">
          <span className="text-[24px] font-bold text-koma-fg">koma</span>
          <span className="text-[12px] text-koma-fg opacity-45">first-run setup</span>
        </div>
        <div className="mb-6 mt-4 flex items-center gap-4">
          <StepDot n={1} active={step === 1} done={step === 2} label="Theme" />
          <div className="h-px w-6 flex-none bg-koma-border" />
          <StepDot n={2} active={step === 2} done={false} label="Connection" />
        </div>

        {step === 1 && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Palette size={12} className="flex-none" />
              Pick a theme
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              Selecting a theme repaints the app instantly. Default is dark — you can change this later.
            </p>
            <div className="grid grid-cols-2 gap-2">
              {themes.map((name) => {
                const active = picked === name
                return (
                  <button
                    key={name}
                    onClick={() => selectTheme(name)}
                    className={`flex items-center justify-between gap-2 rounded-lg border px-3 py-2.5 text-left transition-colors ${
                      active
                        ? 'border-koma-accent bg-koma-accent/10 text-koma-fg'
                        : 'border-koma-border text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
                    }`}
                  >
                    <span className="truncate text-[12.5px]">{titleCase(name)}</span>
                    {active ? (
                      <Check size={14} className="flex-none text-koma-accent" />
                    ) : (
                      <span className="h-3.5 w-3.5 flex-none rounded-full border border-koma-border" />
                    )}
                  </button>
                )
              })}
            </div>
            <div className="mt-6 flex items-center justify-end">
              <button
                onClick={() => setStep(2)}
                className="flex items-center gap-1.5 rounded-lg border border-koma-border px-3.5 py-2 text-[12.5px] text-koma-fg transition-colors hover:border-koma-accent/60 hover:bg-koma-hover"
              >
                Next
                <ArrowRight size={15} className="flex-none" />
              </button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Plug size={12} className="flex-none" />
              {hasProvider ? 'Add a model' : 'Add a provider'}
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              {hasProvider
                ? 'Choose the provider you just added and pick a model to use as your main.'
                : 'Add an API provider (name, base URL, key). Next you’ll pick a model.'}
            </p>

            {/* Reuse the Connector forms verbatim — same GuiReqs, driven by the
                authoritative config push (provider lands -> model form appears). */}
            <div className="flex flex-col rounded-xl border border-koma-border bg-koma-panel">
              {hasProvider ? (
                <ModelForm
                  key={modelDraft.id}
                  draft={modelDraft}
                  providerOptions={providerOptions}
                  onSave={saveModel}
                  onCancel={() => setStep(1)}
                />
              ) : (
                <ProviderForm
                  key={providerDraft.id}
                  draft={providerDraft}
                  onSave={saveProvider}
                  onCancel={() => setStep(1)}
                />
              )}
            </div>

            {/* OAuth — stubbed/greyed for now. */}
            <div className="mt-4 rounded-xl border border-dashed border-koma-border bg-koma-panel2 px-4 py-3 opacity-55">
              <div className="mb-1 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-koma-fg opacity-60">
                <Lock size={12} className="flex-none" />
                Sign in with OAuth
              </div>
              <p className="text-[12px] text-koma-fg opacity-60">
                Connect OpenAI, Anthropic or Kilo Code with your account — coming soon.
              </p>
            </div>

            <div className="mt-6 flex items-center justify-between">
              <button
                onClick={() => setStep(1)}
                className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-[12.5px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <ArrowLeft size={15} className="flex-none" />
                Theme
              </button>
              <span className="text-[11px] text-koma-fg opacity-40">
                {hasProvider ? 'Save a main model to finish' : 'Save a provider to continue'}
              </span>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
