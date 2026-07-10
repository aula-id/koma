import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronRight,
  Loader2,
  Lock,
  Palette,
  Plug,
  Server,
  SlidersHorizontal,
  Sparkles,
} from 'lucide-react'
import { useKoma } from '../store/koma'
import { ProviderForm, PREDEFINED, type ProviderSavePayload } from './panels/connector/ProviderForm'
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

// A preset picked on the "pickProvider" screen, forwarded verbatim into
// ProviderForm's `preset` prop — 'custom' means "blank manual form".
type ProviderPreset = { name: string; endpoint: string } | 'custom'

// FIRST-RUN full-screen onboarding — rendered ahead of the start screen while
// the config has no usable provider/model (host first-run flag, else inferred
// from an empty config). Mirrors the TUI onboarding's step shape, nicer than
// the CLI wizard:
//   1. Theme    — pick a palette from the host registry; selecting it live-
//                 repaints the whole GUI (GuiReq SetTheme -> Config re-push).
//   2. Connect  — a 3-way chooser: Koma Free (keyless, one click), OAuth
//                 (stubbed/greyed), or Provider (API key path).
//   3a. Provider pick — preset marketplace list + Custom (reuses ProviderForm's
//                 PREDEFINED list; onboarding owns the pick screen).
//   3b. Provider form — <ProviderForm> prefilled from the pick (SetProvider).
//   3c. Model form    — <ModelForm> (SetModel, roles ['main'] completes setup).
// Back/cancel always goes exactly one level up; only the chooser's back goes
// all the way to Theme.
export function Onboarding() {
  const themes = useKoma((s) => s.config.themes)
  const activeTheme = useKoma((s) => s.config.theme)
  const providers = useKoma((s) => s.config.providers)
  const oauthProviders = useKoma((s) => s.oauth.providers)
  const oauthConns = useKoma((s) => s.oauth.conns)
  const req = useKoma((s) => s.req)

  // GetOAuthState works pre-session (unlike StartOAuth/SubmitOAuthPaste/
  // CancelOAuth, which are attached-only and a silent no-op this early) — so
  // onboarding CAN show the real, current provider list even though it can't
  // actually start a login flow yet. Fired once on mount.
  useEffect(() => {
    req({ r: 'GetOAuthState' })
  }, [req])

  type Step = 'theme' | 'choose' | 'pickProvider' | 'providerForm' | 'modelForm'

  // An OAuth connection counts as "has a provider" too — it's a fully valid
  // model provider (see providerOptions below), and a user whose ONLY
  // connection is an OAuth one (zero config.providers) must still be able to
  // jump straight to the model step instead of being funneled into "add an
  // API-key provider first".
  const hasProvider = providers.length > 0 || oauthConns.length > 0
  // Edge case: re-entering onboarding with a provider already configured (but
  // no Main model yet, e.g. after a partial setup) — land on the chooser
  // instead of re-picking a theme.
  const [step, setStep] = useState<Step>(() => (hasProvider ? 'choose' : 'theme'))
  // Local highlight source of truth (works even if the host doesn't echo the
  // theme name back on Config). Selecting also fires SetTheme for the live paint.
  const [picked, setPicked] = useState(activeTheme)
  // Which preset (if any) the pickProvider screen sent into the provider form.
  const [providerPreset, setProviderPreset] = useState<ProviderPreset>('custom')
  // "Koma Free" tile pending state — stays disabled until the host's Config
  // re-push flips useNeedsOnboarding and unmounts this component entirely, or
  // until the timeout below gives up on a host that never responds.
  const [komaFreePending, setKomaFreePending] = useState(false)
  const [komaFreeError, setKomaFreeError] = useState(false)
  const komaFreeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // Snapshot of provider count taken the moment we transition INTO
  // 'providerForm' (set in choosePreset). The auto-advance effect below only
  // fires when providers.length has grown past this snapshot — i.e. a new
  // provider was actually saved — never merely because one pre-existed.
  const providerCountOnEntryRef = useRef(providers.length)
  // Which screen led into 'modelForm' — 'choose' when the chooser shortcut
  // straight here (a provider already existed), 'providerForm' when we came
  // through the add-provider flow. Drives the modelForm cancel target.
  const modelFormOriginRef = useRef<'choose' | 'providerForm'>('providerForm')

  const selectTheme = (name: string) => {
    setPicked(name)
    req({ r: 'SetTheme', name })
  }

  const selectKomaFree = () => {
    setKomaFreeError(false)
    setKomaFreePending(true)
    req({ r: 'SetupKomaFree' })
    // No further navigation on success — the host mints the provider+model
    // and re-pushes Config, which completes onboarding (unmounting this
    // component) on its own. If the host never responds, give up after a
    // few seconds so the tile doesn't spin forever.
    if (komaFreeTimeoutRef.current) clearTimeout(komaFreeTimeoutRef.current)
    komaFreeTimeoutRef.current = setTimeout(() => {
      setKomaFreePending(false)
      setKomaFreeError(true)
    }, 8000)
  }

  useEffect(() => {
    return () => {
      if (komaFreeTimeoutRef.current) clearTimeout(komaFreeTimeoutRef.current)
    }
  }, [])

  const choosePreset = (preset: ProviderPreset) => {
    setProviderPreset(preset)
    providerCountOnEntryRef.current = providers.length
    setStep('providerForm')
  }

  // Mirrors ConnectorPanel's providerOptions exactly: the daemon resolves a
  // model's `provider_uuid` against EITHER catalogue — a real config provider
  // OR an OAuth connection (uuid == `OAuthConn.uuid`, routed through that
  // connection's bearer token / chat endpoint) — so an OAuth conn is a fully
  // valid model provider here too, not just inside the Connector panel.
  // OAuth-backed options are label-suffixed "· OAuth" so they're visually
  // distinct from a static API-key provider.
  const providerOptions = useMemo(
    () => [
      ...providers.map((p) => ({ value: p.id, label: p.name })),
      ...oauthConns.map((c) => ({ value: c.uuid, label: `${c.name} · OAuth` })),
    ],
    [providers, oauthConns],
  )

  const saveProvider = (d: ProviderSavePayload) => {
    req({ r: 'SetProvider', uuid: null, name: d.name, endpoint: d.endpoint, apiKey: d.apiKey })
    // Stays on 'providerForm' — the effect below advances to the model form
    // once the provider push lands (hasProvider flips true).
  }
  // Advance 3b -> 3c once the provider we just saved actually lands in the
  // authoritative config. Gated on providers.length growing past the
  // snapshot taken on entry into 'providerForm' — NOT on hasProvider — so a
  // pre-existing provider can never cause a bounce back to 'modelForm' when
  // the user is here specifically to add a new one.
  useEffect(() => {
    if (step === 'providerForm' && providers.length > providerCountOnEntryRef.current) {
      modelFormOriginRef.current = 'providerForm'
      setStep('modelForm')
    }
  }, [step, providers.length])

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

  // Fresh drafts, stable across re-renders of the same sub-step. Re-keyed on
  // the picked preset so switching presets doesn't leak stale typed values.
  const providerDraft = useMemo(
    () => ({ id: nid('prov'), name: '', endpoint: '', hasKey: false }),
    [providerPreset],
  )
  // The model draft pre-selects the (first) provider + seeds the Main role so
  // completing it satisfies the "usable Main" gate immediately. Prefers a real
  // config provider, falling back to the first OAuth connection when that's
  // all the user has (e.g. an OAuth-only setup with zero config.providers).
  const modelDraft = useMemo<Model>(
    () => ({
      id: nid('model'),
      name: '',
      modelId: '',
      provider: providers[0]?.id ?? oauthConns[0]?.uuid ?? '',
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
          <StepDot n={1} active={step === 'theme'} done={step !== 'theme'} label="Theme" />
          <div className="h-px w-6 flex-none bg-koma-border" />
          <StepDot
            n={2}
            active={step === 'choose' || step === 'pickProvider' || step === 'providerForm'}
            done={step === 'modelForm'}
            label="Connect"
          />
          <div className="h-px w-6 flex-none bg-koma-border" />
          <StepDot n={3} active={step === 'modelForm'} done={false} label="Model" />
        </div>

        {step === 'theme' && (
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
                onClick={() => setStep('choose')}
                className="flex items-center gap-1.5 rounded-lg border border-koma-border px-3.5 py-2 text-[12.5px] text-koma-fg transition-colors hover:border-koma-accent/60 hover:bg-koma-hover"
              >
                Next
                <ArrowRight size={15} className="flex-none" />
              </button>
            </div>
          </div>
        )}

        {step === 'choose' && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Plug size={12} className="flex-none" />
              Connect a model
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              Pick how koma should talk to a model. You can change this later.
            </p>

            <div className="flex flex-col gap-2">
              <button
                type="button"
                onClick={selectKomaFree}
                disabled={komaFreePending}
                className={`flex flex-col items-start gap-1 rounded-lg border px-3.5 py-3 text-left transition-colors ${
                  komaFreePending
                    ? 'cursor-default border-koma-border opacity-60'
                    : 'border-koma-border hover:border-koma-accent/60 hover:bg-koma-hover'
                }`}
              >
                <span className="flex items-center gap-1.5 text-[13px] font-semibold text-koma-fg">
                  <Sparkles size={14} className="flex-none text-koma-accent" />
                  Koma Free
                </span>
                {komaFreePending ? (
                  <span className="flex items-center gap-1.5 text-[11.5px] text-koma-fg opacity-60">
                    <Loader2 size={11} className="flex-none animate-spin" />
                    Setting up…
                  </span>
                ) : komaFreeError ? (
                  <span className="text-[11.5px] text-koma-error">Setup failed — try again</span>
                ) : (
                  <span className="text-[11.5px] text-koma-fg opacity-60">
                    Keyless, free tier — start chatting right away.
                  </span>
                )}
              </button>

              <button
                type="button"
                onClick={() => {
                  if (hasProvider) {
                    modelFormOriginRef.current = 'choose'
                    setStep('modelForm')
                  } else {
                    setStep('pickProvider')
                  }
                }}
                className="flex flex-col items-start gap-1 rounded-lg border border-koma-border px-3.5 py-3 text-left transition-colors hover:border-koma-accent/60 hover:bg-koma-hover"
              >
                <span className="flex items-center gap-1.5 text-[13px] font-semibold text-koma-fg">
                  <Plug size={14} className="flex-none text-koma-accent" />
                  Provider
                </span>
                <span className="text-[11.5px] text-koma-fg opacity-60">
                  Bring your own API key (OpenRouter, OpenAI, DeepSeek, and more).
                </span>
              </button>

              {/* OAuth — real, data-driven provider list (never hardcoded), but the
                  login flow itself (StartOAuth/SubmitOAuthPaste/CancelOAuth) is
                  attached-only and silently no-ops with no session — there's
                  nothing actionable to click yet, so this stays a greyed note
                  rather than a working picker. Connect fully from the Connector
                  panel once a session exists. */}
              <div className="flex cursor-not-allowed flex-col items-start gap-1 rounded-lg border border-dashed border-koma-border px-3.5 py-3 opacity-55">
                <span className="flex items-center gap-1.5 text-[13px] font-semibold text-koma-fg">
                  <Lock size={14} className="flex-none" />
                  Sign in with OAuth
                </span>
                <span className="text-[11.5px] text-koma-fg opacity-60">
                  {oauthProviders.length > 0
                    ? `Connect ${oauthProviders
                        .filter((p) => p.kind !== 'paste')
                        .map((p) => p.label)
                        .join(' or ')} with your account — start a session first, then connect from Connector.`
                    : 'Connect with your account — start a session first, then connect from Connector.'}
                </span>
              </div>
            </div>

            <div className="mt-6 flex items-center justify-start">
              <button
                onClick={() => setStep('theme')}
                className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-[12.5px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <ArrowLeft size={15} className="flex-none" />
                Theme
              </button>
            </div>
          </div>
        )}

        {step === 'pickProvider' && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Plug size={12} className="flex-none" />
              Choose a provider
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              Pick a preset (pre-fills the endpoint) or go custom.
            </p>

            <div className="flex flex-col rounded-xl border border-koma-border bg-koma-panel py-1">
              <div className="flex flex-col gap-0.5 px-2 py-1">
                {PREDEFINED.map((p) => (
                  <button
                    key={p.name}
                    type="button"
                    onClick={() => choosePreset(p)}
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
                  onClick={() => choosePreset('custom')}
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

            <div className="mt-6 flex items-center justify-start">
              <button
                onClick={() => setStep('choose')}
                className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-[12.5px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <ArrowLeft size={15} className="flex-none" />
                Back
              </button>
            </div>
          </div>
        )}

        {step === 'providerForm' && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Plug size={12} className="flex-none" />
              Add a provider
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              Add an API provider (name, base URL, key). Next you’ll pick a model.
            </p>

            <div className="flex flex-col rounded-xl border border-koma-border bg-koma-panel">
              <ProviderForm
                key={typeof providerPreset === 'string' ? providerPreset : providerPreset.name}
                draft={providerDraft}
                preset={providerPreset}
                onSave={saveProvider}
                onCancel={() => setStep('pickProvider')}
              />
            </div>

            <div className="mt-6 flex items-center justify-between">
              <span className="text-[11px] text-koma-fg opacity-40">Save a provider to continue</span>
            </div>
          </div>
        )}

        {step === 'modelForm' && (
          <div className="flex flex-1 flex-col">
            <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
              <Plug size={12} className="flex-none" />
              Add a model
            </div>
            <p className="mb-3 text-[12px] text-koma-fg opacity-55">
              Choose the provider you just added and pick a model to use as your main.
            </p>

            <div className="flex flex-col rounded-xl border border-koma-border bg-koma-panel">
              <ModelForm
                key={modelDraft.id}
                draft={modelDraft}
                providerOptions={providerOptions}
                onSave={saveModel}
                onCancel={() => setStep(modelFormOriginRef.current === 'choose' ? 'choose' : 'pickProvider')}
              />
            </div>

            <div className="mt-6 flex items-center justify-between">
              <span className="text-[11px] text-koma-fg opacity-40">Save a main model to finish</span>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
