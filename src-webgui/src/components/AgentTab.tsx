import { useEffect, useMemo, useRef, useState } from 'react'
import { Check, Trash2, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { Chips, Field, Select, TextInput } from './panels/form'

type AgentTabProps = {
  tab: Extract<Tab, { kind: 'agent' }>
}

// Per-agent editor tab — create (tab.agentId === null) or edit (tab.agentId =
// the agent's CURRENT name). Settings-tab visual language (full-height
// koma-bg page, centered content column, every colour a koma-* theme token)
// but single-column (one entity, not multiple browsable sections), reusing
// the panels/form.tsx field primitives (Field/TextInput/Select/Segmented) the
// MCP/Connector forms already use.
export default function AgentTab({ tab }: AgentTabProps) {
  const req = useKoma((s) => s.req)
  const agents = useKoma((s) => s.agents)
  const catalogueModels = useKoma((s) => s.catalogueModels)
  const catalogueProviders = useKoma((s) => s.catalogueProviders)
  const availableTools = useKoma((s) => s.availableTools)
  const agentSaving = useKoma((s) => s.agentSaving)
  const setAgentSaving = useKoma((s) => s.setAgentSaving)
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  const agentId = tab.agentId // null = create
  const isCreate = agentId === null
  const existing = agents.find((a) => a.name === agentId)
  const isBuiltin = existing?.source === 'builtin'

  const [name, setName] = useState(existing?.name ?? '')
  const [description, setDescription] = useState(existing?.description ?? '')
  const [conditions, setConditions] = useState(existing?.conditions ?? '')
  const [modelUuid, setModelUuid] = useState<string | null>(existing?.modelUuid ?? null)
  const [tools, setTools] = useState<string[]>(existing?.tools ?? [])
  const [prompt, setPrompt] = useState(existing?.prompt ?? '')
  const [scope, setScope] = useState<'global' | 'session'>('session')
  const [armedDelete, setArmedDelete] = useState(false)
  // Local validation error messages. Cleared when the field value changes
  // (inline in the onChange handler) and re-computed on save if still invalid.
  const [nameError, setNameError] = useState<string | null>(null)
  const [descError, setDescError] = useState<string | null>(null)

  // Whether THIS tab's save is in flight — derived from the store's agentSaving
  // tracker by matching on the client-local tab id (not agentId), so a different
  // tab's save never disables this one.
  const isSaving = agentSaving?.tabId === tab.id

  // Re-hydrate ONLY when the tab's identity (agentId) actually changes — a
  // genuine switch to a different agent, or the rebind after a successful
  // create/rename (renameAgentTab flips `agentId`, see koma.ts). Never on
  // every unrelated `agents` update (some OTHER agent/session mutation),
  // which would clobber in-progress edits here.
  //
  // `hydratedFor` is a ONE-SHOT marker per identity: it's set the FIRST time
  // this effect sees a given agentId, whether or not the matching entry has
  // arrived yet. Without that, an identity change right after a save
  // (the entry isn't in `agents` yet — this tab's own SetAgent reply
  // hasn't landed as a push) would leave the marker unadvanced, so the SAME
  // effect re-fires on every later `agents` update until the entry finally
  // shows up — at which point it would unconditionally overwrite whatever the
  // user had typed in the meantime. Marking it done immediately makes this
  // genuinely one-shot: at most one hydrate attempt per identity, ever.
  //
  // `dirty` additionally guards that single attempt: any field edit sets it
  // true, Save clears it. If the user has already typed into this identity by
  // the time the (one) hydrate attempt runs, their state wins outright — we
  // never fetch-and-overwrite over an in-progress edit.
  const hydratedFor = useRef<string | null>(agentId)
  const dirty = useRef(false)
  useEffect(() => {
    if (hydratedFor.current === agentId) return
    hydratedFor.current = agentId
    if (dirty.current) return
    const a = agents.find((x) => x.name === agentId)
    if (!a && agentId !== null) return
    setName(a?.name ?? '')
    setDescription(a?.description ?? '')
    setConditions(a?.conditions ?? '')
    setModelUuid(a?.modelUuid ?? null)
    setTools(a?.tools ?? [])
    setPrompt(a?.prompt ?? '')
  }, [agentId, agents])

  // Danger/error palette role tint — same lookup ToastContainer/SessionRowActions
  // use (index 9 of the 11-role PaletteInfo.colors array). Never a hardcoded
  // red/orange.
  const errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])

  // "name @ provider" options, resolved client-side from the catalogues (per
  // the locked design — not trusting the wire's own pre-resolved `model`
  // string), plus the null "(inherit main)" option every agent (including
  // every builtin, which never carries a model override) can land on.
  const modelOptions = useMemo(
    () => [
      { value: '', label: '(inherit main)' },
      ...catalogueModels.map((m) => {
        const provider = catalogueProviders.find((p) => p.uuid === m.providerUuid)
        return { value: m.uuid, label: provider ? `${m.name} @ ${provider.name}` : m.name }
      }),
    ],
    [catalogueModels, catalogueProviders],
  )

  // Toggle-chip options for the Tools field: every daemon-known tool, PLUS
  // any tool this agent's saved `tools` already carries that ISN'T in that
  // list (a stale/custom entry) — appended so it still renders as a selected
  // chip and saving never silently drops it.
  const toolOptions = useMemo(() => {
    const extra = tools.filter((t) => !availableTools.includes(t))
    return [...availableTools, ...extra].map((t) => ({ value: t, label: t }))
  }, [availableTools, tools])

  // Derived validation: the name field must be non-empty after trimming.
  const trimmedName = name.trim()
  const trimmedDesc = description.trim()
  const canSave = !isSaving && trimmedName.length > 0 && trimmedDesc.length > 0

  const save = () => {
    // Client-side validation (visible inline via the error state below).
    const nOk = trimmedName.length > 0
    const dOk = trimmedDesc.length > 0
    if (!nOk) setNameError('Name is required')
    else setNameError(null)
    if (!dOk) setDescError('Description is required')
    else setDescError(null)
    if (!nOk || !dOk) return

    // This save commits the currently-typed state, so any "typed since" flag
    // is moot going forward — clear it before the identity-changing rebind
    // below so the one-shot hydration effect is free to (harmlessly) settle
    // once the confirming AgentsValues push lands, matching what we just sent.
    dirty.current = false
    // `scope` is a required wire field either way, but the daemon only truly
    // USES it for a CREATE — on an edit it derives the real write tier from
    // the agent's own current source (a builtin edit auto-becomes a session
    // override), falling back to this value only if the named agent no
    // longer exists. Mirror that same derivation client-side for a sane
    // fallback rather than sending a stale/meaningless value.
    const effectiveScope: 'global' | 'session' = isCreate
      ? scope
      : existing?.source === 'global'
        ? 'global'
        : 'session'

    // Track this save request BEFORE firing so the store can correlate the
    // confirming AgentsValues/AgentOp push. NO premature renameAgentTab here:
    // the tab's agentId stays unchanged until the authoritative push confirms
    // the save landed, preventing stale/raced rebinds.
    const seq = setAgentSaving(tab.id, agentId, trimmedName)
    req({
      r: 'SetAgent',
      originalName: agentId,
      scope: effectiveScope,
      name: trimmedName,
      description: trimmedDesc,
      conditions,
      modelUuid: modelUuid || null,
      tools,
      prompt,
      reqSeq: seq,
    })
  }

  const confirmDelete = () => {
    if (agentId === null) return
    req({ r: 'DeleteAgent', scope: existing?.source === 'global' ? 'global' : 'session', name: agentId })
    setArmedDelete(false)
    // No local close needed — the next AgentsValues push will no longer list
    // this agent, and the store's push handler closes the tab for us.
  }

  // Shared styling for the validation error message chip — same visual as
  // the "delete forever?" chip in the footer.
  const errorChip = (msg: string) => (
    <span
      className="mt-0.5 inline-flex items-center gap-1 rounded px-1.5 py-px text-[10px] font-medium"
      style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
    >
      {msg}
    </span>
  )

  return (
    <div className="flex h-full w-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-1 px-8 py-6">
          <div className="mb-4 border-b border-koma-border pb-2">
            <h2 className="text-[15px] font-semibold text-koma-fg">
              {isCreate ? 'New agent' : name || agentId}
            </h2>
            <p className="mt-0.5 text-[12px] text-koma-fg opacity-45">
              {isCreate
                ? 'Define a sub-agent the main agent can delegate to.'
                : 'Renaming keeps this tab pointed at the agent.'}
            </p>
          </div>

          <Field label="Name">
            <TextInput
              value={name}
              autoFocus={isCreate}
              onChange={(e) => {
                dirty.current = true
                setName(e.target.value)
                if (nameError) setNameError(null)
              }}
              placeholder="e.g. code-reviewer"
            />
            {nameError && errorChip(nameError)}
          </Field>

          <Field label="Description">
            <TextInput
              value={description}
              onChange={(e) => {
                dirty.current = true
                setDescription(e.target.value)
                if (descError) setDescError(null)
              }}
              placeholder="required — shown in the delegation picker"
            />
            {descError && errorChip(descError)}
          </Field>

          <Field label="Conditions">
            <textarea
              value={conditions}
              onChange={(e) => {
                dirty.current = true
                setConditions(e.target.value)
              }}
              rows={2}
              placeholder="when should the main agent delegate to this?"
              className="w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
            />
          </Field>

          <Field label="Model">
            <Select
              value={modelUuid ?? ''}
              options={modelOptions}
              onChange={(v) => {
                dirty.current = true
                setModelUuid(v || null)
              }}
            />
          </Field>

          <Field label="Tools">
            {/* Empty selection is valid — the daemon's own semantics treat it
                as "default/read-only tools", not "no tools allowed" (see
                AgentEntry's comment in koma.ts) — no validation invented here. */}
            <Chips
              value={tools}
              options={toolOptions}
              onToggle={(t) => {
                dirty.current = true
                setTools((cur) => (cur.includes(t) ? cur.filter((x) => x !== t) : [...cur, t]))
              }}
            />
          </Field>

          {isCreate ? (
            <Field label="Scope">
              <Select
                value={scope}
                onChange={setScope}
                options={[
                  { value: 'session', label: 'Session' },
                  { value: 'global', label: 'Global' },
                ]}
              />
            </Field>
          ) : (
            <Field label="Scope">
              {/* `existing` is briefly undefined right after a create-save's
                  confirming push (the AgentsValues push hasn't landed yet) —
                  fall back to the scope the user actually picked in the create
                  form rather than defaulting to "Session" and flashing the wrong
                  tier for a moment. Once `existing` resolves this switches to
                  the authoritative value (self-corrects either way). */}
              <span className="text-[12px] text-koma-fg opacity-60">
                {existing
                  ? existing.source === 'global'
                    ? 'Global'
                    : existing.source === 'builtin'
                      ? 'Built-in'
                      : 'Session'
                  : scope === 'global'
                    ? 'Global'
                    : 'Session'}
              </span>
            </Field>
          )}

          <Field label="Prompt">
            <textarea
              value={prompt}
              onChange={(e) => {
                dirty.current = true
                setPrompt(e.target.value)
              }}
              rows={16}
              spellCheck={false}
              placeholder="the sub-agent's system prompt"
              className="min-h-[320px] w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11.5px] leading-relaxed text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
            />
          </Field>

          {isBuiltin && (
            <p className="px-3 py-1.5 text-[11px] text-koma-fg opacity-45">
              built-in — saving creates a session override
            </p>
          )}
        </div>
      </div>

      <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-4 py-2.5">
        {!isCreate &&
          !isBuiltin &&
          (armedDelete ? (
            <span
              className="flex items-center gap-1.5 rounded px-2 py-1 text-[12px] font-medium"
              style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
            >
              delete forever?
              <button
                onClick={confirmDelete}
                disabled={isSaving}
                aria-label="Confirm delete"
                className="flex items-center gap-1 rounded px-1.5 font-semibold opacity-90 transition-opacity hover:opacity-100 disabled:opacity-40"
                style={{ color: errorTint }}
              >
                <Check size={13} className="flex-none" />
                yes
              </button>
              <button
                onClick={() => setArmedDelete(false)}
                aria-label="Cancel delete"
                className="flex items-center gap-1 rounded px-1.5 text-koma-fg opacity-70 transition-opacity hover:opacity-100"
              >
                <X size={13} className="flex-none" />
                no
              </button>
            </span>
          ) : (
            <button
              onClick={() => setArmedDelete(true)}
              disabled={isSaving}
              className="flex items-center gap-1.5 rounded px-2.5 py-1 text-[12px] opacity-80 transition-opacity hover:opacity-100 disabled:opacity-40"
              style={{ color: errorTint }}
            >
              <Trash2 size={13} className="flex-none" />
              Delete
            </button>
          ))}
        <button
          onClick={save}
          disabled={!canSave}
          className="flex items-center gap-1.5 rounded border border-koma-border px-3 py-1.5 text-[12px] font-medium text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
        >
          {isSaving ? (
            <>
              <BrailleSpinner size={12} />
              Saving
            </>
          ) : (
            'Save'
          )}
        </button>
      </div>
    </div>
  )
}
