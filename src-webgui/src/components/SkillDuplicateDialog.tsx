import { useEffect, useMemo, useRef, useState } from 'react'
import type { SkillCatalogueEntry, SkillScope } from '../store/koma'
import { useKoma } from '../store/koma'
import { insideSkillProject } from './skillProject'
import { BrailleSpinner } from './BrailleSpinner'

let duplicateSeq = 0
const validName = /^[a-z0-9][a-z0-9_-]*$/

type Props = { skills: SkillCatalogueEntry[]; onClose: () => void }

export function SkillDuplicateDialog({ skills, onClose }: Props) {
  const req = useKoma((s) => s.req)
  const catalogue = useKoma((s) => s.skills)
  const epoch = useKoma((s) => s.skillSessionEpoch)
  const sessionId = useKoma((s) => s.session.id)
  const workdirs = useKoma((s) => s.settingsValues?.workdir)
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const insideProject = insideSkillProject(sessionId, workdirs, activeRoot)
  const lastOp = useKoma((s) => s.skillLastOp)
  const openSkillTab = useKoma((s) => s.openSkillTab)
  const [scope, setScope] = useState<Exclude<SkillScope, 'external'>>('global')
  const [targets, setTargets] = useState<Record<string, string>>(() =>
    Object.fromEntries(skills.map((skill) => [skill.skillId, `${skill.name}-copy`])),
  )
  const [requestId, setRequestId] = useState<string | null>(null)
  const restoreFocus = useRef<HTMLElement | null>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const closeRef = useRef(onClose)
  const busyRef = useRef(false)
  closeRef.current = onClose
  busyRef.current = Boolean(requestId && lastOp?.requestId !== requestId)

  useEffect(() => {
    restoreFocus.current = document.activeElement as HTMLElement | null
    cancelRef.current?.focus()
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busyRef.current) closeRef.current()
      if (event.key === 'Tab') {
        const dialog = cancelRef.current?.closest('[role="dialog"]')
        const focusable = [...(dialog?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [])]
        const first = focusable[0]
        const last = focusable[focusable.length - 1]
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
      }
    }
    window.addEventListener('keydown', key)
    return () => { window.removeEventListener('keydown', key); restoreFocus.current?.focus() }
  }, [])

  const existing = useMemo(() => new Set(catalogue.map((skill) => skill.name)), [catalogue])
  const rows = skills.map((source) => ({ source, target: targets[source.skillId] ?? '' }))
  const invalid = rows.some(({ target }) => !validName.test(target) || existing.has(target)) || (scope === 'project' && !insideProject)
  const result = requestId && lastOp?.requestId === requestId ? lastOp : null
  const complete = Boolean(result)

  useEffect(() => {
    if (!insideProject && scope === 'project') setScope('global')
  }, [insideProject, scope])

  useEffect(() => {
    if (!result) return
    const firstSuccess = result.outcomes.find((outcome) => outcome.status === 'success')
    if (!firstSuccess) return
    const destination = rows.find(({ source }) => source.name === firstSuccess.name)?.target
    const duplicate = catalogue.find((skill) => skill.name === destination)
    if (duplicate) openSkillTab(duplicate.skillId, duplicate.name)
  }, [catalogue, openSkillTab, result])

  const submit = () => {
    if (invalid) return
    const id = `duplicate-${++duplicateSeq}`
    setRequestId(id)
    req({
      r: 'DuplicateSkills',
      target: scope,
      items: rows.map(({ source, target }) => ({ skillId: source.skillId, generation: source.generation, name: source.name, destinationName: target })),
      requestId: id,
      sessionEpoch: epoch,
      tabId: 'duplicate-dialog',
    })
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-3" role="dialog" aria-modal="true" aria-labelledby="duplicate-skills-title">
      <div className="w-full max-w-lg rounded border border-koma-border bg-koma-panel shadow-xl">
        <div className="border-b border-koma-border px-4 py-3">
          <h2 id="duplicate-skills-title" className="text-[13px] font-semibold">Duplicate to Koma</h2>
          <p className="mt-0.5 text-[11px] opacity-55">Creates an owned copy. Source files remain unchanged.</p>
        </div>
        <div className="max-h-[55vh] space-y-3 overflow-auto px-4 py-3">
          <div className="flex gap-1" aria-label="Target scope">
            {(['global', 'project'] as const).map((value) => <button type="button" key={value} disabled={complete || (value === 'project' && !insideProject)} title={value === 'project' && !insideProject ? 'Open a project to duplicate into Project scope' : undefined} onClick={() => setScope(value)} aria-pressed={scope === value} className={`rounded border px-2.5 py-1 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40 ${scope === value ? 'border-koma-accent bg-koma-accent/15' : 'border-koma-border'}`}>{value === 'global' ? 'Global' : 'Project'}</button>)}
          </div>
          {rows.map(({ source, target }) => {
            const outcome = result?.outcomes.find((item) => item.name === source.name)
            return <label key={source.skillId} className="block text-[11px]">
              <span className="mb-1 block opacity-60">{source.name} →</span>
              <input value={target} disabled={Boolean(requestId)} onChange={(event) => setTargets((current) => ({ ...current, [source.skillId]: event.target.value.trim().toLowerCase() }))} aria-invalid={!validName.test(target) || existing.has(target)} className="h-8 w-full rounded border border-koma-border bg-koma-bg px-2 outline-none focus:border-koma-accent" />
              {!validName.test(target) && <span className="mt-1 block text-red-400">Use lowercase letters, numbers, hyphens, or underscores.</span>}
              {existing.has(target) && !outcome && <span className="mt-1 block text-red-400">A skill with this name already exists.</span>}
              {outcome && <span className={`mt-1 block ${outcome.status === 'success' ? 'text-green-400' : 'text-red-400'}`}>{outcome.status === 'success' ? 'Duplicated' : outcome.error ?? outcome.status}</span>}
            </label>
          })}
          {!insideProject && <p className="text-[11px] text-koma-fg opacity-45">Open a project to duplicate into Project scope. Global stays available.</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-koma-border px-4 py-3">
          <button ref={cancelRef} type="button" onClick={onClose} disabled={Boolean(requestId) && !complete} className="rounded px-2.5 py-1 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40">{complete ? 'Close' : 'Cancel'}</button>
          {!complete && <button type="button" onClick={submit} disabled={invalid || Boolean(requestId)} className="flex items-center gap-1.5 rounded bg-koma-accent px-3 py-1 text-[12px] font-semibold text-koma-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40">{requestId && <BrailleSpinner size={12} />} Duplicate {skills.length} skill{skills.length === 1 ? '' : 's'}</button>}
        </div>
      </div>
    </div>
  )
}
