import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ChevronRight, FilePlus, Info, Plus, Search, Sparkles, Upload, X, type LucideIcon } from 'lucide-react'
import { useKoma, type SkillCatalogueEntry } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'
import { DetailHeader, Empty } from './helpers'
import { Segmented, Select } from './form'
import { SkillDeleteConfirm } from '../SkillDeleteConfirm'
import { SkillDuplicateDialog } from '../SkillDuplicateDialog'
import { nextSkillSelection } from '../skillListSelection'
import { insideSkillProject, skillProjectRoot } from '../skillProject'

type SkillsPanelView = 'list' | 'methods'

const EMPTY_ROOTS: string[] = []

function rootLabel(root: string): string {
  const parts = root.split(/[/\\]/).filter(Boolean)
  return parts[parts.length - 1] || root
}

function underWorkspace(path: string, root: string): boolean {
  const file = path.replace(/\\/g, '/').replace(/\/+$/, '')
  const base = root.replace(/\\/g, '/').replace(/\/+$/, '')
  return file === base || file.startsWith(`${base}/`)
}

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

type SkillMenuState = { x: number; y: number; skillIds: string[] }

function SkillContextMenu({
  state,
  skills,
  sessionId,
  loaded,
  onClose,
  onLoad,
  onRemove,
  onReload,
  onDuplicate,
  onDelete,
}: {
  state: SkillMenuState
  skills: SkillCatalogueEntry[]
  sessionId: string | null
  loaded: Set<string>
  onClose: () => void
  onLoad: (skills: SkillCatalogueEntry[]) => void
  onRemove: (skills: SkillCatalogueEntry[]) => void
  onReload: (skills: SkillCatalogueEntry[]) => void
  onDuplicate: (skills: SkillCatalogueEntry[]) => void
  onDelete: (skills: SkillCatalogueEntry[]) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: state.x, top: state.y })
  const loadedSkills = skills.filter((skill) => loaded.has(skill.name))
  const unloadedSkills = skills.filter((skill) => !loaded.has(skill.name))
  const owned = skills.filter((skill) => skill.scope !== 'external')
  const external = skills.filter((skill) => skill.scope === 'external')
  const onlyExternal = external.length > 0 && external.length === skills.length
  const onlyOwned = owned.length > 0 && owned.length === skills.length

  useEffect(() => {
    const el = ref.current
    if (el) {
      setPos({
        left: Math.max(4, Math.min(state.x, window.innerWidth - el.offsetWidth - 4)),
        top: Math.max(4, Math.min(state.y, window.innerHeight - el.offsetHeight - 4)),
      })
    }
    const outside = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', outside, true)
    window.addEventListener('keydown', key)
    return () => {
      window.removeEventListener('mousedown', outside, true)
      window.removeEventListener('keydown', key)
    }
  }, [onClose, state.x, state.y])

  const item = 'flex w-full items-center px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-35'
  const run = (action: (skills: SkillCatalogueEntry[]) => void, targets: SkillCatalogueEntry[]) => {
    action(targets)
    onClose()
  }

  return createPortal(
    <div
      ref={ref}
      role="menu"
      data-tour="skills-context-menu"
      style={{ position: 'fixed', ...pos, width: 196, zIndex: 95 }}
      className="overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
      onContextMenu={(event) => event.preventDefault()}
    >
      <div className="truncate px-2.5 py-1 text-[10px] text-koma-dim">
        {skills.length > 1 ? `${skills.length} selected` : skills[0]?.name}
      </div>
      <button type="button" role="menuitem" disabled={!sessionId || !unloadedSkills.length} title={!sessionId ? 'Open a chat to load skills into it' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} className={item} onClick={() => run(onLoad, unloadedSkills)}>Load into chat</button>
      <button type="button" role="menuitem" disabled={!sessionId || !loadedSkills.length} title={!sessionId ? 'Open a chat to remove skills from it' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} className={item} onClick={() => run(onRemove, loadedSkills)}>Remove from chat</button>
      <button type="button" role="menuitem" disabled={!sessionId || !loadedSkills.length} title={!sessionId ? 'Open a chat to reload skills from disk' : undefined} aria-describedby={!sessionId ? 'skills-no-active-chat' : undefined} className={item} onClick={() => run(onReload, loadedSkills)}>Reload from disk</button>
      <div className="my-1 border-t border-koma-border" />
      <button type="button" role="menuitem" disabled={!onlyExternal} title={onlyExternal ? 'Duplicate selected External skills' : 'Only External skills can be duplicated to Koma'} className={item} onClick={() => run(onDuplicate, external)}>Duplicate</button>
      <button type="button" role="menuitem" disabled={!onlyOwned} title={onlyOwned ? 'Delete selected skills' : 'External skills cannot be deleted'} className={item} onClick={() => run(onDelete, owned)}>Delete</button>
    </div>,
    document.body,
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
  const workdirs = useKoma((s) => s.settingsValues?.workdir ?? EMPTY_ROOTS)
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const setActiveCodingRoot = useKoma((s) => s.setActiveCodingRoot)
  const setSelection = useKoma((s) => s.setSkillSelection)
  const openSkillTab = useKoma((s) => s.openSkillTab)
  const openUploadSkillTab = useKoma((s) => s.openUploadSkillTab)
  const refillComposer = useKoma((s) => s.refillComposer)
  const activateTab = useKoma((s) => s.activateTab)
  const req = useKoma((s) => s.req)
  const searchRef = useRef<HTMLInputElement>(null)
  const rowRefs = useRef(new Map<string, HTMLButtonElement>())
  const anchorRef = useRef<string | null>(null)
  const discoveredEpochRef = useRef<number | null>(null)
  const [view, setView] = useState<SkillsPanelView>('list')
  const [focusId, setFocusId] = useState<string | null>(null)
  const [menu, setMenu] = useState<SkillMenuState | null>(null)
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

  const projectRoot = skillProjectRoot(workdirs, activeRoot)
  const insideProject = insideSkillProject(sessionId, workdirs, activeRoot)

  useEffect(() => {
    if (!insideProject && view === 'methods') setView('list')
  }, [insideProject, view])

  useEffect(() => {
    if (filter !== 'project' || workdirs.length === 0) return
    if (!activeRoot || !workdirs.includes(activeRoot)) setActiveCodingRoot(workdirs[0])
  }, [filter, workdirs, activeRoot, setActiveCodingRoot])

  const scannedWorkspace = useRef<string | null>(null)
  useEffect(() => {
    if (filter !== 'project' || !sessionId || !projectRoot) return
    if (scannedWorkspace.current === projectRoot) return
    scannedWorkspace.current = projectRoot
    refreshSkills()
  }, [filter, sessionId, projectRoot, refreshSkills])

  const loaded = useMemo(() => new Set(loadedNames), [loadedNames])
  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase()
    const matched = skills.filter((skill) => {
      const tab = skill.scope === 'project' || skill.sourceTier === 'claude' ? 'project' : 'global'
      if (tab !== filter) return false
      if (filter === 'project' && projectRoot && skill.sourcePath && !underWorkspace(skill.sourcePath, projectRoot)) return false
      return !needle || [skill.name, skill.description, skill.triggers]
        .some((value) => value.toLocaleLowerCase().includes(needle))
    })
    return matched.slice().sort((a, b) => {
      const rank = (name: string) => (loaded.has(name) ? 0 : 1)
      const byLoaded = rank(a.name) - rank(b.name)
      if (byLoaded !== 0) return byLoaded
      return a.name.localeCompare(b.name)
    })
  }, [filter, loaded, projectRoot, query, skills])
  const visibleIds = useMemo(() => visible.map((skill) => skill.skillId), [visible])
  const selectedSet = useMemo(() => new Set(selection), [selection])
  const menuSkills = useMemo(
    () => (menu ? visible.filter((skill) => menu.skillIds.includes(skill.skillId)) : []),
    [menu, visible],
  )

  useEffect(() => {
    const kept = selection.filter((id) => visibleIds.includes(id))
    if (kept.length !== selection.length) setSelection(kept)
    if (anchorRef.current && !visibleIds.includes(anchorRef.current)) anchorRef.current = null
    if (focusId && !visibleIds.includes(focusId)) setFocusId(visibleIds[0] ?? null)
  }, [focusId, selection, setSelection, visibleIds])

  const closeMenu = () => setMenu(null)

  const choose = (skill: SkillCatalogueEntry, event: React.MouseEvent<HTMLButtonElement>) => {
    // Embedded WebViews may leave DOM focus elsewhere after a pointer click.
    // Make the clicked row the actual arrow-key starting point.
    event.currentTarget.focus()
    setFocusId(skill.skillId)
    if (event.ctrlKey || event.metaKey || event.shiftKey) {
      const next = nextSkillSelection(visibleIds, selection, anchorRef.current, skill.skillId, event)
      setSelection(next.selected)
      anchorRef.current = next.anchor
      return
    }
    openSkill(skill)
  }

  const openMenu = (skill: SkillCatalogueEntry, event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault()
    event.stopPropagation()
    const skillIds = selectedSet.has(skill.skillId)
      ? visibleIds.filter((id) => selectedSet.has(id))
      : [skill.skillId]
    setMenu({ x: event.clientX, y: event.clientY, skillIds })
    setFocusId(skill.skillId)
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
    else if (event.key === 'Enter' && currentId) {
      const skill = skills.find((item) => item.skillId === currentId)
      if (skill) openSkill(skill)
      return
    } else return
    event.preventDefault()
    const id = visibleIds[next]
    setFocusId(id)
    rowRefs.current.get(id)?.focus()
  }

  if (view === 'methods' && insideProject) {
    const methods: { title: string; subtitle: string; icon: LucideIcon; onClick: () => void }[] = [
      {
        title: 'Create new',
        subtitle: 'Global or Project skill',
        icon: FilePlus,
        onClick: () => { openSkillTab(null); setView('list') },
      },
      {
        title: 'Upload .zip',
        subtitle: 'Install a packaged skill',
        icon: Upload,
        onClick: () => { openUploadSkillTab(); setView('list') },
      },
      {
        title: 'Create with Koma',
        subtitle: 'Start a guided chat draft',
        icon: Sparkles,
        onClick: () => {
          if (!insideProject) return
          refillComposer(SKILL_CREATION_TEMPLATE)
          activateTab('chat')
          setView('list')
        },
      },
    ]
    return (
      <div data-tour="skills-panel" className="flex h-full min-w-0 flex-col overflow-hidden bg-koma-panel text-koma-fg">
        <DetailHeader onBack={() => setView('list')} title="Add skill" backTourId="skills-add-back" tourId="skills-add-header" />
        <div data-tour="skills-add-methods" className="min-h-0 flex-1 overflow-auto py-1">
          <div className="px-3 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
            Choose a method
          </div>
          <div className="flex flex-col gap-0.5 px-2">
            {methods.map((method) => (
              <button
                key={method.title}
                type="button"
                onClick={method.onClick}
                className="flex items-center gap-2 rounded px-2 py-1.5 text-left transition-colors hover:bg-koma-hover"
              >
                <method.icon size={14} className="flex-none text-koma-accent" />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="text-[12.5px] text-koma-fg">{method.title}</span>
                  <span className="truncate text-[10.5px] text-koma-fg opacity-40">{method.subtitle}</span>
                </span>
                <ChevronRight size={13} className="flex-none text-koma-fg opacity-30" />
              </button>
            ))}
          </div>
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
      <div className="flex-none px-2 pb-1.5 pt-1.5">
        <div data-tour="skills-filters" aria-label="Skill filter">
          <Segmented
            value={filter}
            onChange={setFilter}
            options={[
              { value: 'global', label: 'Global' },
              {
                value: 'project',
                label: 'Project',
                disabled: !sessionId,
                title: !sessionId ? 'Open a chat to see Project skills' : undefined,
                describedBy: !sessionId ? 'skills-no-active-chat' : undefined,
              },
            ]}
          />
        </div>
      </div>
      {filter === 'project' && (
        <div className="flex flex-none items-center gap-1 px-2 pb-1.5">
          <div className="min-w-0 flex-1" title={projectRoot}>
            <Select
              value={projectRoot}
              options={workdirs.map((root) => ({ value: root, label: rootLabel(root) }))}
              onChange={(root) => setActiveCodingRoot(root)}
              disabled={!sessionId || workdirs.length === 0}
              placeholder={sessionId ? 'No workspace' : 'Open a chat'}
            />
          </div>
        </div>
      )}
      <div className="flex-none border-b border-koma-border px-2 pb-2">
        <div className="relative">
          <Search size={12} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 opacity-45" />
          <input data-tour="skills-search" ref={searchRef} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search skills…" aria-label="Search skills" className="h-7 min-w-0 w-full rounded border border-koma-border bg-koma-bg pl-7 pr-7 text-[11px] text-koma-fg outline-none placeholder:text-koma-dim focus:border-koma-grip" />
          {query && <button type="button" onClick={() => setQuery('')} aria-label="Clear search" className="absolute right-1.5 top-1/2 -translate-y-1/2 opacity-50 hover:opacity-100"><X size={12} /></button>}
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
          : !visible.length ? <Empty>{query ? `No skills match “${query}”` : filter === 'project' ? 'No project skills' : 'No global skills'}</Empty>
          : visible.map((skill, index) => {
            const isSelected = selectedSet.has(skill.skillId)
            const isOpen = activeSkillId === skill.skillId
            const isLoaded = loaded.has(skill.name)
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
                onClick={(event) => choose(skill, event)}
                onDoubleClick={() => openSkill(skill)}
                onContextMenu={(event) => openMenu(skill, event)}
                className={`group flex min-h-[42px] w-full min-w-0 items-start border-l-2 px-3 py-1.5 text-left hover:bg-koma-hover ${isLoaded ? 'border-l-koma-accent' : 'border-l-transparent'} ${isSelected ? 'bg-koma-head' : ''}`}
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] text-koma-fg">{skill.name}</span>
                  {skill.description && <span className="block truncate text-[10px] text-koma-fg opacity-45">{skill.description}</span>}
                </span>
                {isLoaded && <span data-skill-marker="loaded" className="ml-2 mt-0.5 flex-none self-start rounded-full bg-koma-accent/15 px-1.5 py-0.5 text-[9px] font-medium text-koma-accent">active</span>}
                <span className="sr-only">item {index + 1} of {visible.length}</span>
              </button>
            )
          })}
      </div>

      <div className="flex-none border-t border-koma-border p-2"><button type="button" data-tour="skills-add" disabled={!insideProject} title={insideProject ? 'Add skill' : 'Open a project to add a skill'} onClick={() => { if (insideProject) setView('methods') }} className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:bg-transparent"><Plus size={13} /> Add skill</button></div>
      {menu && menuSkills.length > 0 && (
        <SkillContextMenu
          state={menu}
          skills={menuSkills}
          sessionId={sessionId}
          loaded={loaded}
          onClose={closeMenu}
          onLoad={(targets) => operate('SetSkillsLoaded', targets.map((skill) => skill.name), true)}
          onRemove={(targets) => operate('SetSkillsLoaded', targets.map((skill) => skill.name), false)}
          onReload={(targets) => operate('ReloadSkills', targets.map((skill) => skill.name))}
          onDuplicate={setDuplicate}
          onDelete={setDeleting}
        />
      )}
      {lastOp?.tabId === 'skills-panel' && <span className="sr-only" role="status" aria-live="polite">{lastOp.outcomes.map((outcome) => `${outcome.name}: ${outcome.status}${outcome.error ? `, ${outcome.error}` : ''}`).join('. ')}</span>}
      {duplicate && <SkillDuplicateDialog skills={duplicate} onClose={() => setDuplicate(null)} />}
      {deleting && <SkillDeleteConfirm skills={deleting} onClose={() => setDeleting(null)} />}
    </div>
  )
}
