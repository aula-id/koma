import { useEffect, useMemo, useRef, useState } from 'react'
import { Bot, Code2, Copy, Download, Eye, RefreshCw, Trash2 } from 'lucide-react'
import { useKoma, type Tab } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { MessageBody } from './MessageBody'
import { SkillDeleteConfirm } from './SkillDeleteConfirm'
import { SkillDuplicateDialog } from './SkillDuplicateDialog'
import { Chips, Field, Select, TextInput } from './panels/form'

type Props = { tab: Extract<Tab, { kind: 'skill' }> }
let mutationSeq = 0

export default function SkillTab({ tab }: Props) {
  const skills = useKoma((s) => s.skills)
  const skillsLoading = useKoma((s) => s.skillsLoading)
  const details = useKoma((s) => s.skillDetails)
  const pending = useKoma((s) => s.skillDetailPending)
  const detailErrors = useKoma((s) => s.skillDetailErrors)
  const files = useKoma((s) => s.skillFiles)
  const loadedNames = useKoma((s) => s.loadedSkillNames)
  const epoch = useKoma((s) => s.skillSessionEpoch)
  const sessionId = useKoma((s) => s.session.id)
  const opResults = useKoma((s) => s.skillOpResults)
  const availableTools = useKoma((s) => s.availableTools)
  const req = useKoma((s) => s.req)
  const requestDetail = useKoma((s) => s.requestSkillDetail)
  const readFile = useKoma((s) => s.readSkillFile)
  const refreshSkills = useKoma((s) => s.refreshSkills)
  const closeTab = useKoma((s) => s.closeTab)
  const openSkillTab = useKoma((s) => s.openSkillTab)
  const refillComposer = useKoma((s) => s.refillComposer)
  const newSessionPreservingTabs = useKoma((s) => s.newSessionPreservingTabs)
  const activateTab = useKoma((s) => s.activateTab)

  const entry = tab.skillId ? skills.find((skill) => skill.skillId === tab.skillId) : undefined
  const detail = details[tab.id]
  const isCreate = tab.skillId === null
  const [scope, setScope] = useState<'global' | 'project'>('global')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [triggers, setTriggers] = useState('')
  const [tools, setTools] = useState<string[]>([])
  const [instruction, setInstruction] = useState('')
  const [preview, setPreview] = useState(false)
  const [activeFile, setActiveFile] = useState<string | null>(null)
  const [requestId, setRequestId] = useState<string | null>(null)
  const [saveAction, setSaveAction] = useState<'save' | 'save-reload' | null>(null)
  const [requestTimedOut, setRequestTimedOut] = useState(false)
  const requestEpoch = useRef<number | null>(null)
  const [downloadRequestId, setDownloadRequestId] = useState<string | null>(null)
  const [duplicate, setDuplicate] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const hydratedGeneration = useRef<string | null>(null)
  const editGeneration = useRef<string | null>(null)
  const dirty = useRef(false)

  useEffect(() => {
    if (availableTools.length > 0) return
    req({ r: 'GetAgents' })
  }, [availableTools.length, req])

  useEffect(() => {
    if (!sessionId && scope === 'project') setScope('global')
  }, [sessionId, scope])

  useEffect(() => {
    if (isCreate || !entry || detail || pending[tab.id]) return
    requestDetail(tab.id, entry.skillId, entry.generation)
  }, [detail, entry, isCreate, pending, requestDetail, tab.id])

  useEffect(() => {
    if (!entry || !detail || dirty.current || pending[tab.id] || entry.generation === detail.generation) return
    requestDetail(tab.id, entry.skillId, entry.generation)
  }, [detail, entry, pending, requestDetail, tab.id])

  useEffect(() => {
    if (!detail || hydratedGeneration.current === detail.generation || dirty.current) return
    hydratedGeneration.current = detail.generation
    editGeneration.current = detail.generation
    setName(detail.name)
    setDescription(detail.description)
    setTriggers(detail.triggers)
    setTools(detail.declaredTools)
    setInstruction(detail.instruction)
  }, [detail])

  const ownResult = requestId ? opResults[requestId] ?? null : null
  const success = ownResult?.outcomes.some((outcome) => outcome.status === 'success') ?? false
  const failure = ownResult?.outcomes.find((outcome) => outcome.status === 'failed')?.error
  const partial = ownResult?.outcomes.find((outcome) => outcome.status === 'partial')?.error
  const downloadResult = downloadRequestId ? opResults[downloadRequestId] ?? null : null
  const downloadSuccess = downloadResult?.outcomes.find((outcome) => outcome.status === 'success')
  const downloadFailure = downloadResult?.outcomes.find((outcome) => outcome.status === 'failed')?.error

  useEffect(() => {
    if (!success && !partial) return
    dirty.current = false
    if (isCreate) {
      const created = skills.find((skill) => skill.name === name.trim())
      if (created) {
        closeTab(tab.id)
        openSkillTab(created.skillId, created.name)
      }
      return
    }
  }, [closeTab, detail?.generation, entry, isCreate, name, openSkillTab, partial, pending, requestDetail, skills, success, tab.id])

  const toolOptions = useMemo(() => {
    const extra = tools.filter((tool) => !availableTools.includes(tool))
    return [...availableTools, ...extra].map((tool) => ({ value: tool, label: tool }))
  }, [availableTools, tools])

  const external = detail?.scope === 'external'
  const sourceUnavailable = Boolean(!isCreate && detail && !entry)
  const sourceChanged = Boolean(detail && entry && editGeneration.current && entry.generation !== editGeneration.current)
  const stale = sourceUnavailable || sourceChanged
  const loaded = Boolean(detail && loadedNames.includes(detail.name))
  const validCreateName = /^[a-z0-9][a-z0-9_-]*$/.test(name.trim())
  const canSave = Boolean(!stale && name.trim() && (!isCreate || validCreateName) && description.trim() && (isCreate || (detail?.editable && detail.structuredSaveSupported)))
  const busy = Boolean(requestId && !ownResult)
  const confirmationMissing = busy && (requestTimedOut || (requestEpoch.current !== null && requestEpoch.current !== epoch))
  useEffect(() => {
    if (!busy || confirmationMissing) return
    const timeout = window.setTimeout(() => setRequestTimedOut(true), 15_000)
    return () => window.clearTimeout(timeout)
  }, [busy, confirmationMissing])
  const companionKey = activeFile ? `${tab.id}:${activeFile}` : null
  const companion = companionKey ? files[companionKey] : undefined

  useEffect(() => {
    if (!activeFile || !detail || companion || pending[tab.id]) return
    readFile(tab.id, detail.skillId, detail.generation, activeFile)
  }, [activeFile, companion, detail, pending, readFile, tab.id])

  const mutate = (kind: 'create' | 'update', reloadAfterSave = false) => {
    if (!canSave || busy || (reloadAfterSave && (!loaded || !sessionId))) return
    const id = `${kind}-${++mutationSeq}`
    requestEpoch.current = epoch
    setSaveAction(reloadAfterSave ? 'save-reload' : 'save')
    setRequestTimedOut(false)
    setRequestId(id)
    if (kind === 'create') {
      req({ r: 'CreateSkill', target: scope, name: name.trim(), description: description.trim(), triggers, allowedTools: tools, instruction, requestId: id, sessionEpoch: epoch, tabId: tab.id })
    } else if (detail) {
      req({ r: 'UpdateSkill', skillId: detail.skillId, generation: editGeneration.current ?? detail.generation, name: detail.name, description: description.trim(), triggers, allowedTools: tools, instruction, reloadAfterSave, targetSessionId: reloadAfterSave ? sessionId ?? undefined : undefined, requestId: id, sessionEpoch: epoch, tabId: tab.id })
    }
  }

  const editWithKoma = () => {
    if (!detail) return
    const sourceRule = external
      ? 'This source is External and read-only. Do not modify it. Help me design the revision; I will Duplicate to Koma and apply it to the editable duplicate.'
      : 'Help me design the revision; I will reopen this skill and save the agreed version.'
    const template = `Help me edit the Koma skill "${detail.name}".

Exact source path: ${detail.sourcePath}
Read the exact source before proposing changes and preserve unknown or advanced YAML fields.
${sourceRule}

Current supported metadata:
- description: ${detail.description}
- triggers: ${detail.triggers || '(none)'}
- allowed-tools: ${detail.declaredTools.length ? detail.declaredTools.join(', ') : '(none)'}

Current Prompt / Instruction:
${detail.instruction}

What I want to change:
<describe the change>`
    if (!sessionId) newSessionPreservingTabs()
    refillComposer(template)
    activateTab('chat')
  }

  const downloadZip = () => {
    if (!detail || external || stale) return
    const id = `download-${++mutationSeq}`
    setDownloadRequestId(id)
    req({ r: 'DownloadSkillZip', skillId: detail.skillId, generation: editGeneration.current ?? detail.generation, name: detail.name, requestId: id, sessionEpoch: epoch, tabId: tab.id })
  }

  const contextOp = (r: 'SetSkillsLoaded' | 'ReloadSkills', loadedValue?: boolean) => {
    if (!detail) return
    const id = `${r}-${++mutationSeq}`
    requestEpoch.current = epoch
    setSaveAction(null)
    setRequestTimedOut(false)
    setRequestId(id)
    const common = { names: [detail.name], requestId: id, sessionEpoch: epoch, tabId: tab.id }
    if (r === 'ReloadSkills') req({ r, ...common })
    else req({ r, loaded: Boolean(loadedValue), ...common })
  }

  if (!isCreate && !detail && (pending[tab.id] || (skillsLoading && !entry))) return <div className="flex h-full items-center justify-center"><BrailleSpinner size={16} /></div>
  if (!isCreate && !detail) return <div className="flex h-full flex-col items-center justify-center gap-2 text-[12px] opacity-60"><span>{detailErrors[tab.id] || 'Skill no longer available.'}</span><button type="button" onClick={refreshSkills} className="rounded border border-koma-border px-2 py-1">Refresh</button></div>

  const companions = detail?.companionFiles ?? []
  return (
    <div className="flex h-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-1 px-8 py-6">
          <div className="mb-4 border-b border-koma-border pb-2">
            <h2 className="text-[15px] font-semibold text-koma-fg">{isCreate ? 'New skill' : detail?.name}</h2>
            <p className="mt-0.5 text-[12px] text-koma-fg opacity-45">{isCreate ? 'Create a Global or Project skill.' : external ? 'External skill — read-only; duplicate it to Koma to customize.' : `${detail?.scope === 'project' ? 'Project' : 'Global'} skill.`}</p>
          </div>

          {!isCreate && detail?.editable && !detail.structuredSaveSupported && <div className="rounded border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200">This skill uses advanced YAML. Structured Save is disabled to prevent data loss; use Edit with Koma or edit SKILL.md manually.</div>}
          {stale && detail && <div className="flex items-center justify-between gap-3 rounded border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200"><span>{sourceUnavailable ? 'This source is no longer the current catalogue winner. The draft is preserved read-only and cannot be saved.' : 'The source changed on disk while this draft was open. The draft is preserved read-only and saving is disabled.'}</span>{entry && <button type="button" onClick={() => { dirty.current = false; hydratedGeneration.current = null; requestDetail(tab.id, entry.skillId, entry.generation) }} className="flex-none rounded border border-amber-500/30 px-2 py-1">Discard draft and refresh</button>}</div>}
          {success && <div role="status" className="rounded border border-green-500/25 bg-green-500/10 px-3 py-2 text-[11px] text-green-300">{ownResult?.operation === 'update-reload' ? 'Saved and reloaded in this chat.' : loaded && ownResult?.operation === 'update' ? 'Saved. Reload to update the current chat context.' : `${ownResult?.operation ?? 'Operation'} completed.`}</div>}
          {partial && <div role="alert" className="rounded border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200">{partial}</div>}
          {failure && <div role="alert" className="rounded border border-red-500/25 bg-red-500/10 px-3 py-2 text-[11px] text-red-300">{failure}</div>}
          {confirmationMissing && <div role="alert" className="rounded border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200">Koma did not confirm this operation. It may already have changed the file; check the disk version before trying again. Rescan skill locations and reopen the skill to verify.</div>}
          {downloadSuccess && <div role="status" className="rounded border border-green-500/25 bg-green-500/10 px-3 py-2 text-[11px] text-green-300">Saved ZIP to {downloadSuccess.name}</div>}
          {downloadFailure && <div role="alert" className="rounded border border-red-500/25 bg-red-500/10 px-3 py-2 text-[11px] text-red-300">{downloadFailure}</div>}

          <Field label="Name">
            <TextInput value={name} disabled={!isCreate} autoFocus={isCreate} onChange={(event) => { dirty.current = true; setName(event.target.value.toLowerCase()) }} aria-invalid={isCreate && name.length > 0 && !validCreateName} placeholder="e.g. review-notes" />
            {isCreate && name.length > 0 && !validCreateName && <span className="mt-0.5 text-[10px] text-koma-fg opacity-70">Use lowercase letters, numbers, hyphens, or underscores.</span>}
          </Field>
          {isCreate && (
            <Field label="Scope">
              <Select
                value={scope}
                onChange={setScope}
                options={sessionId ? [{ value: 'global', label: 'Global' }, { value: 'project', label: 'Project' }] : [{ value: 'global', label: 'Global' }]}
              />
              {!sessionId && <span className="text-[11px] text-koma-fg opacity-45">Open a chat to create a Project skill.</span>}
            </Field>
          )}
          <Field label="Description">
            <TextInput value={description} readOnly={external || stale} onChange={(event) => { dirty.current = true; setDescription(event.target.value) }} placeholder="required — shown in the skill catalogue" />
          </Field>
          <Field label="Triggers">
            <TextInput value={triggers} readOnly={external || stale} onChange={(event) => { dirty.current = true; setTriggers(event.target.value) }} placeholder="when should this skill be used?" />
          </Field>
          <Field label="Tools">
            <p className="text-[11px] text-koma-fg opacity-45">Saved as allowed-tools. Koma does not currently enforce these restrictions.</p>
            <div className={external || stale ? 'pointer-events-none opacity-55' : undefined}>
              <Chips
                value={tools}
                options={toolOptions}
                onToggle={(tool) => {
                  if (external || stale) return
                  dirty.current = true
                  setTools((current) => (current.includes(tool) ? current.filter((item) => item !== tool) : [...current, tool]))
                }}
              />
            </div>
          </Field>

          {!isCreate && <div className="flex min-w-0 gap-1 overflow-x-auto border-b border-koma-border" role="tablist"><button type="button" role="tab" aria-selected={activeFile === null} onClick={() => setActiveFile(null)} className={`flex-none px-2 py-1 text-[11px] ${activeFile === null ? 'border-b-2 border-koma-accent' : 'opacity-50'}`}>SKILL.md</button>{companions.map((path) => <button type="button" role="tab" aria-selected={activeFile === path} key={path} onClick={() => setActiveFile(path)} className={`flex-none px-2 py-1 text-[11px] ${activeFile === path ? 'border-b-2 border-koma-accent' : 'opacity-50'}`}>{path}</button>)}</div>}

          {activeFile ? <Field label={activeFile}><div className="mb-1 flex justify-end"><div className="flex rounded border border-koma-border p-0.5"><button type="button" onClick={() => setPreview(false)} aria-pressed={!preview} className={`flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] ${!preview ? 'bg-koma-hover' : 'opacity-50'}`}><Code2 size={11} /> Code</button><button type="button" onClick={() => setPreview(true)} aria-pressed={preview} className={`flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] ${preview ? 'bg-koma-hover' : 'opacity-50'}`}><Eye size={11} /> Preview</button></div></div>{pending[tab.id] && !companion ? <BrailleSpinner size={14} /> : companion?.error ? <div role="alert" className="text-[11px] text-koma-fg opacity-70">{companion.error}</div> : preview ? <div className="min-h-[260px] rounded border border-koma-border bg-koma-bg p-3 text-[12px]">{companion?.content.trim() ? <MessageBody text={companion.content} /> : <span className="opacity-45">Empty file.</span>}</div> : <pre className="max-h-[440px] overflow-auto whitespace-pre-wrap rounded border border-koma-border bg-koma-bg p-3 font-mono text-[11px]">{companion?.content ?? ''}</pre>}</Field> : <Field label="Prompt"><div className="mb-1 flex justify-end"><div className="flex rounded border border-koma-border p-0.5"><button type="button" onClick={() => setPreview(false)} aria-pressed={!preview} className={`flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] ${!preview ? 'bg-koma-hover' : 'opacity-50'}`}><Code2 size={11} /> Code</button><button type="button" onClick={() => setPreview(true)} aria-pressed={preview} className={`flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] ${preview ? 'bg-koma-hover' : 'opacity-50'}`}><Eye size={11} /> Preview</button></div></div>{preview ? <div className="min-h-[260px] rounded border border-koma-border bg-koma-bg p-3 text-[12px]">{instruction.trim() ? <MessageBody text={instruction} /> : <span className="opacity-45">Nothing to preview yet.</span>}</div> : <textarea value={instruction} readOnly={external || stale} onChange={(event) => { dirty.current = true; setInstruction(event.target.value) }} rows={16} spellCheck={false} placeholder="the skill instruction" className="min-h-[320px] w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11.5px] leading-relaxed text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip read-only:opacity-70" />}</Field>}
        </div>
      </div>

      <footer className="flex flex-none flex-wrap items-center justify-end gap-2 border-t border-koma-border px-4 py-2.5">
        {!isCreate && detail && <>{external && <button type="button" onClick={() => setDuplicate(true)} disabled={stale} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] disabled:opacity-35"><Copy size={12} /> Duplicate to Koma</button>}<button type="button" onClick={editWithKoma} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px]"><Bot size={12} /> Edit with Koma</button>{!external && <button type="button" onClick={downloadZip} disabled={stale} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] disabled:opacity-35"><Download size={12} /> Download .zip</button>}{!external && <button type="button" onClick={() => setDeleting(true)} disabled={stale} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] text-red-300 disabled:opacity-35"><Trash2 size={12} /> Delete</button>}<button type="button" title={loaded ? 'Reload from disk' : 'Load into chat'} onClick={() => contextOp(loaded ? 'ReloadSkills' : 'SetSkillsLoaded', true)} disabled={busy || stale} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] disabled:opacity-35">{loaded && <RefreshCw size={12} />}{loaded ? 'Reload' : 'Load'}</button>{loaded && <button type="button" onClick={() => contextOp('SetSkillsLoaded', false)} disabled={busy || stale} className="rounded border border-koma-border px-2 py-1 text-[11px] disabled:opacity-35">Unload</button>}</>}
        {!external && <button type="button" onClick={() => mutate(isCreate ? 'create' : 'update')} disabled={!canSave || busy} className="flex items-center gap-1 rounded bg-koma-head px-3 py-1 text-[11px] disabled:opacity-35">{busy && saveAction === 'save' && !confirmationMissing && <BrailleSpinner size={12} />} Save</button>}
        {!isCreate && !external && loaded && sessionId && <button type="button" onClick={() => mutate('update', true)} disabled={!canSave || busy} className="flex items-center gap-1 rounded bg-koma-head px-3 py-1 text-[11px] disabled:opacity-35">{busy && saveAction === 'save-reload' && !confirmationMissing && <BrailleSpinner size={12} />} Save &amp; Reload</button>}
      </footer>
      {duplicate && detail && entry && <SkillDuplicateDialog skills={[entry]} onClose={() => setDuplicate(false)} />}
      {deleting && entry && <SkillDeleteConfirm skills={[entry]} onClose={() => setDeleting(false)} />}
    </div>
  )
}
