import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowLeft, Info, Lock, Plus, RefreshCw, Search, X } from 'lucide-react'
import { useKoma, type SkillCatalogueEntry } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'
import { Empty } from './helpers'
import { SkillDeleteConfirm } from '../SkillDeleteConfirm'
import { SkillDuplicateDialog } from '../SkillDuplicateDialog'
import { nextSkillSelection } from '../skillListSelection'

type SkillsPanelView = 'list' | 'methods'

const SKILL_CREATION_TEMPLATE = `Help me create a new Koma skill. A skill is a SKILL.md file with YAML frontmatter (description, optional triggers, optional allowed-tools) plus a Markdown instruction body.

What the skill should do:
<describe the task or workflow>

When it should trigger:
<describe when it should be used>

Prompt / instruction:
<describe the detailed instructions>

Work through the design with me. When it is ready, I will use the Skills panel's Create new form to save it.`

let operationSeq = 0
function operationId(kind: string) {
  operationSeq += 1
  return `${kind}-${operationSeq}`
}

function ScopeBadge({ scope }: { scope: SkillCatalogueEntry['scope'] }) {
  return (
    <span className="flex flex-none items-center gap-0.5 rounded bg-koma-head px-1 py-px text-[9px] uppercase tracking-wide text-koma-fg opacity-60">
      {scope === 'external' && <Lock size={9} aria-hidden="true" />}
      {scope}
    </span>
  )
}

export function SkillsPanel() {
  const skills = useKoma((s) => s.skills)
  const loadedNames = useKoma((s) => s.loadedSkillNames)
  const loading = useKoma((s) => s.skillsLoading)
  const error = useKoma((s) => s.skillsError)
  const unconfirmed = useKoma((s) => s.skillsUnconfirmed)
  const query = useKoma((s) => s.skillQuery)
  const filter = useKoma((s) => s.skillFilter)
  const selection = useKoma((s) => s.skillSelection)
  const epoch = useKoma((s) => s.skillSessionEpoch)
  const sessionId = useKoma((s) => s.session.id)
  const preserveTabsOnNextSession = useKoma((s) => s.ui.preserveTabsOnNextSession)
  const preservedTabsTargetSession = useKoma((s) => s.ui.preservedTabsTargetSession)
  const activeSkillId = useKoma((s) => {
    const activeTab = s.ui.tabs.find((tab) => tab.id === s.ui.activeTabId)
    return activeTab?.kind === 'skill' ? activeTab.skillId : null
  })
  const lastOp = useKoma((s) => s.skillLastOp)
  const refreshSkills = useKoma((s) => s.refreshSkills)
  const setQuery = useKoma((s) => s.setSkillQuery)
  const setFilter = useKoma((s) => s.setSkillFilter)
  const setSelection = useKoma((s) => s.setSkillSelection)
  const openSkillTab = useKoma((s) => s.openSkillTab)
  const openUploadSkillTab = useKoma((s) => s.openUploadSkillTab)
  const refillComposer = useKoma((s) => s.refillComposer)
  const newSessionPreservingTabs = useKoma((s) => s.newSessionPreservingTabs)
  const activateTab = useKoma((s) => s.activateTab)
  const req = useKoma((s) => s.req)
  const searchRef = useRef<HTMLInputElement>(null)
  const rowRefs = useRef(new Map<string, HTMLButtonElement>())
  const anchorRef = useRef<string | null>(null)
  const discoveredEpochRef = useRef<number | null>(null)
  const [view, setView] = useState<SkillsPanelView>('list')
  const [focusId, setFocusId] = useState<string | null>(null)
  const [duplicate, setDuplicate] = useState<SkillCatalogueEntry[] | null>(null)
  const [deleting, setDeleting] = useState<SkillCatalogueEntry[] | null>(null)

  useEffect(() => {
    if (discoveredEpochRef.current === epoch) return
    // A guided first-chat switch may receive an unrelated Snapshot before the
    // target is known. Detached browsing still discovers Global/External skills.
    if (sessionId && preserveTabsOnNextSession && preservedTabsTargetSession !== sessionId) return
    discoveredEpochRef.current = epoch
    // A retained Skill tab may already be rediscovering this same epoch.
    // skillRequestId alone is not pending: it survives success and timeout.
    if (!useKoma.getState().skillsLoading) refreshSkills()
  }, [epoch, refreshSkills, sessionId, preserveTabsOnNextSession, preservedTabsTargetSession])

  const loaded = useMemo(() => new Set(loadedNames), [loadedNames])
  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase()
    return skills.filter((skill) => {
      if (filter === 'loaded' && !loaded.has(skill.name)) return false
      return !needle || [skill.name, skill.description, skill.triggers]
        .some((value) => value.toLocaleLowerCase().includes(needle))
    })
  }, [filter, loaded, query, skills])
  const visibleIds = useMemo(() => visible.map((skill) => skill.skillId), [visible])
  const selectedSet = useMemo(() => new Set(selection), [selection])
  const selected = visible.filter((skill) => selectedSet.has(skill.skillId))
  const selectedLoaded = selected.filter((skill) => loaded.has(skill.name))
  const selectedUnloaded = selected.filter((skill) => !loaded.has(skill.name))
  const selectedOwned = selected.filter((skill) => skill.scope !== 'external')
  const selectedExternal = selected.filter((skill) => skill.scope === 'external')

  useEffect(() => {
    const kept = selection.filter((id) => visibleIds.includes(id))
    if (kept.length !== selection.length) setSelection(kept)
    if (anchorRef.current && !visibleIds.includes(anchorRef.current)) anchorRef.current = null
    if (focusId && !visibleIds.includes(focusId)) setFocusId(visibleIds[0] ?? null)
  }, [focusId, selection, setSelection, visibleIds])

  const choose = (skillId: string, event: React.MouseEvent<HTMLButtonElement>) => {
    // Embedded WebViews may leave DOM focus elsewhere after a pointer click.
    // Make the clicked row the actual arrow-key starting point.
    event.currentTarget.focus()
    const next = nextSkillSelection(visibleIds, selection, anchorRef.current, skillId, event)
    setSelection(next.selected)
    anchorRef.current = next.anchor
    setFocusId(skillId)
  }

  const operate = (r: 'SetSkillsLoaded' | 'ReloadSkills', names: string[], loadedValue?: boolean) => {
    if (!sessionId || !names.length) return
    const requestId = operationId(r)
    const common = { requestId, sessionEpoch: epoch, tabId: 'skills-panel' }
    if (r === 'ReloadSkills') req({ r, names, ...common })
    else req({ r, names, loaded: Boolean(loadedValue), ...common })
  }

  const openSkill = (skill: SkillCatalogueEntry) => {
    setSelection([])
    anchorRef.current = null
    openSkillTab(skill.skillId, skill.name)
  }

  const onListKeyDown = (event: React.KeyboardEvent) => {
    if (!visibleIds.length) return
    const eventSkillId = (event.target as HTMLElement).closest<HTMLElement>('[data-skill-id]')?.dataset.skillId
    const currentId = eventSkillId ?? focusId
    const current = currentId ? visibleIds.indexOf(currentId) : -1
    let next = current
    if (event.key === 'ArrowDown') next = Math.min(visibleIds.length - 1, current + 1)
    else if (event.key === 'ArrowUp') next = Math.max(0, current < 0 ? 0 : current - 1)
    else if (event.key === 'Home') next = 0
    else if (event.key === 'End') next = visibleIds.length - 1
    else if (event.key === 'Enter' && focusId) {
      const skill = skills.find((item) => item.skillId === focusId)
      if (skill) openSkill(skill)
      return
    } else return
    event.preventDefault()
    const id = visibleIds[next]
    setFocusId(id)
    rowRefs.current.get(id)?.focus()
  }

  if (view === 'methods') {
    return (
      <div data-tour="skills-panel" className="flex h-full min-w-0 flex-col overflow-hidden bg-koma-panel text-koma-fg">
        <div className="flex-none border-b border-koma-border px-2 py-2">
          <button type="button" data-tour="skills-add-back" onClick={() => setView('list')} className="flex items-center gap-1 rounded px-1 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"><ArrowLeft size={13} /> Add skill</button>
        </div>
        <div data-tour="skills-add-methods" className="min-h-0 flex-1 space-y-1.5 overflow-auto p-2">
          <button type="button" onClick={() => { openSkillTab(null); setView('list') }} className="w-full rounded border border-koma-border px-2.5 py-2 text-left text-[11px] text-koma-fg hover:border-koma-accent hover:bg-koma-hover"><span className="block font-semibold">Create new</span><span className="block text-[10px] text-koma-dim">Global or Project skill</span></button>
          <button type="button" onClick={() => { openUploadSkillTab(); setView('list') }} className="w-full rounded border border-koma-border px-2.5 py-2 text-left text-[11px] text-koma-fg hover:border-koma-accent hover:bg-koma-hover"><span className="block font-semibold">Upload .zip</span><span className="block text-[10px] text-koma-dim">Install a packaged skill</span></button>
          <button type="button" onClick={() => { if (!sessionId) newSessionPreservingTabs(); refillComposer(SKILL_CREATION_TEMPLATE); activateTab('chat'); setView('list') }} className="w-full rounded border border-koma-border px-2.5 py-2 text-left text-[11px] text-koma-fg hover:border-koma-accent hover:bg-koma-hover"><span className="block font-semibold">Create with Koma</span><span className="block text-[10px] text-koma-dim">Start a guided chat draft</span></button>
        </div>
      </div>
    )
  }

  return (
    <div data-tour="skills-panel" className="flex h-full min-w-0 flex-col overflow-hidden text-koma-fg" onKeyDown={(event) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
        event.preventDefault()
        searchRef.current?.focus()
      } else if (event.key === 'Escape') {
        if (selection.length) setSelection([])
        else if (query) setQuery('')
      }
    }}>
      <div className="flex-none border-b border-koma-border px-2 py-2">
        <div className="relative">
          <Search size={12} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 opacity-45" />
          <input data-tour="skills-search" ref={searchRef} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search skills…" aria-label="Search skills" className="h-7 min-w-0 w-full rounded border border-koma-border bg-koma-bg pl-7 pr-7 text-[11px] text-koma-fg outline-none placeholder:text-koma-dim focus:border-koma-grip" />
          {query && <button type="button" onClick={() => setQuery('')} aria-label="Clear search" className="absolute right-1.5 top-1/2 -translate-y-1/2 opacity-50 hover:opacity-100"><X size={12} /></button>}
        </div>
        <div className="mt-1.5 flex min-w-0 items-center gap-0.5">
          <div data-tour="skills-filters" className="flex min-w-0 flex-1 items-center gap-0.5" aria-label="Skill filter">
            <button type="button" onClick={() => setFilter('all')} aria-pressed={filter === 'all'} className={`flex-none rounded px-1 py-0.5 text-[9px] ${filter === 'all' ? 'bg-koma-head' : 'opacity-50 hover:opacity-80'}`}>All</button>
            <button type="button" onClick={() => setFilter('loaded')} disabled={!sessionId} title={!sessionId ? 'Open a chat to filter skills loaded into it' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} aria-pressed={filter === 'loaded'} className={`min-w-0 whitespace-nowrap rounded px-1 py-0.5 text-[9px] disabled:cursor-not-allowed ${filter === 'loaded' ? 'bg-koma-head' : 'opacity-50 hover:opacity-80'} disabled:opacity-30`}>Loaded in chat</button>
          </div>
          <button type="button" onClick={refreshSkills} disabled={loading} aria-label="Rescan skill locations" title="Rescan skill locations" className="flex h-6 w-5 flex-none items-center justify-center rounded opacity-60 hover:bg-koma-hover hover:opacity-100 disabled:cursor-wait">{loading ? <BrailleSpinner size={12} /> : <RefreshCw size={12} />}</button>
        </div>
      </div>

      {!sessionId && (
        <div id="skills-no-active-chat" data-testid="skills-no-active-chat" role="note" className="flex flex-none gap-1.5 border-b border-koma-border bg-koma-head/35 px-2 py-2 text-[10px] leading-relaxed text-koma-dim">
          <Info size={12} className="mt-0.5 flex-none text-koma-accent" aria-hidden="true" />
          <span><strong className="font-semibold text-koma-fg">No active chat.</strong> Open a chat to discover Project skills and load skills into chat.</span>
        </div>
      )}

      {unconfirmed && <div role="status" className="flex-none border-b border-koma-border px-2 py-1.5 text-[10px] text-koma-dim">{unconfirmed}</div>}

      <div data-tour="skills-list" role="listbox" aria-label="Skills" aria-multiselectable="true" className="min-h-0 flex-1 overflow-auto py-1" onKeyDown={onListKeyDown}>
        {loading && !skills.length ? <div className="flex justify-center py-10"><BrailleSpinner size={16} /></div>
          : error ? <Empty>{error}</Empty>
          : !visible.length ? <Empty>{query ? `No skills match “${query}”` : filter === 'loaded' ? 'No skills loaded' : 'No skills installed'}</Empty>
          : visible.map((skill, index) => {
            const isSelected = selectedSet.has(skill.skillId)
            const isOpen = activeSkillId === skill.skillId
            return (
              <button
                key={skill.skillId}
                ref={(node) => { if (node) rowRefs.current.set(skill.skillId, node); else rowRefs.current.delete(skill.skillId) }}
                type="button"
                data-skill-id={skill.skillId}
                role="option"
                aria-selected={isSelected}
                aria-current={isOpen ? 'true' : undefined}
                tabIndex={(focusId ?? visibleIds[0]) === skill.skillId ? 0 : -1}
                onFocus={() => setFocusId(skill.skillId)}
                onClick={(event) => choose(skill.skillId, event)}
                onDoubleClick={() => openSkill(skill)}
                className={`relative flex min-h-[46px] w-full min-w-0 items-center px-2 py-1.5 text-left transition-[background-color] focus:outline focus:outline-2 focus:-outline-offset-2 focus:outline-koma-accent ${isSelected ? 'bg-koma-head' : isOpen ? 'bg-koma-hover' : 'hover:bg-koma-hover'}`}
              >
                {(isSelected || isOpen) && (
                  <span
                    aria-hidden="true"
                    data-skill-marker={isSelected ? 'selected' : 'open'}
                    className={`absolute left-0 w-0.5 bg-koma-accent ${isSelected ? 'inset-y-0' : 'inset-y-1.5 rounded-r'}`}
                  />
                )}
                <span className="min-w-0 flex-1">
                  <span className="flex min-w-0 items-center gap-1">
                    <span className="truncate text-[12px] text-koma-fg">{skill.name}</span>
                    <ScopeBadge scope={skill.scope} />
                    {isOpen && <span className="flex-none rounded bg-koma-accent/15 px-1 py-px text-[9px] uppercase tracking-wide text-koma-accent">Open</span>}
                    {loaded.has(skill.name) && <span className="flex-none rounded bg-koma-accent/15 px-1 py-px text-[9px] uppercase tracking-wide text-koma-accent">Loaded</span>}
                  </span>
                  {skill.description && <span className="block truncate text-[10px] text-koma-fg opacity-45">{skill.description}</span>}
                </span>
                <span className="sr-only">item {index + 1} of {visible.length}</span>
              </button>
            )
          })}
      </div>

      {selected.length ? (
        <div data-tour="skills-bulk" className="flex-none border-t border-koma-border bg-koma-panel p-2 text-[10px]" aria-live="polite">
          <div className="mb-1 flex items-center justify-between"><span>{selected.length} selected</span><button type="button" onClick={() => setSelection([])} aria-label="Clear selection"><X size={12} /></button></div>
          <div className="flex flex-wrap gap-1">
            <button type="button" disabled={!sessionId || !selectedUnloaded.length} title={!sessionId ? 'Open a chat to load skills into it' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} onClick={() => operate('SetSkillsLoaded', selectedUnloaded.map((skill) => skill.name), true)} className="rounded border border-koma-border px-1.5 py-1 disabled:opacity-30">Load into chat</button>
            <button type="button" disabled={!sessionId || !selectedLoaded.length} title={!sessionId ? 'Open a chat to remove skills from it' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} onClick={() => operate('SetSkillsLoaded', selectedLoaded.map((skill) => skill.name), false)} className="rounded border border-koma-border px-1.5 py-1 disabled:opacity-30">Remove from chat</button>
            <button type="button" disabled={!sessionId || !selectedLoaded.length} title={!sessionId ? 'Open a chat to reload skills from disk' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} onClick={() => operate('ReloadSkills', selectedLoaded.map((skill) => skill.name))} className="rounded border border-koma-border px-1.5 py-1 disabled:opacity-30">Reload from disk</button>
            <button type="button" disabled={!selectedExternal.length || selectedExternal.length !== selected.length} title={selectedExternal.length !== selected.length ? 'Only External skills can be duplicated to Koma' : 'Duplicate selected External skills'} onClick={() => setDuplicate(selectedExternal)} className="rounded border border-koma-border px-1.5 py-1 disabled:opacity-30">Duplicate</button>
            <button type="button" disabled={!selectedOwned.length || selectedOwned.length !== selected.length} title={selectedOwned.length !== selected.length ? 'External skills cannot be deleted' : 'Delete selected skills'} onClick={() => setDeleting(selectedOwned)} className="rounded border border-koma-border px-1.5 py-1 disabled:opacity-30">Delete</button>
          </div>
        </div>
      ) : (
        <div className="flex-none border-t border-koma-border p-2"><button type="button" data-tour="skills-add" onClick={() => setView('methods')} className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"><Plus size={13} /> Add skill</button></div>
      )}
      {lastOp?.tabId === 'skills-panel' && <span className="sr-only" role="status" aria-live="polite">{lastOp.outcomes.map((outcome) => `${outcome.name}: ${outcome.status}${outcome.error ? `, ${outcome.error}` : ''}`).join('. ')}</span>}
      {duplicate && <SkillDuplicateDialog skills={duplicate} onClose={() => setDuplicate(null)} />}
      {deleting && <SkillDeleteConfirm skills={deleting} onClose={() => setDeleting(null)} />}
    </div>
  )
}
