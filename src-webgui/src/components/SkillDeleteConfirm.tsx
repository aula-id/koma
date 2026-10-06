import { useEffect, useRef, useState } from 'react'
import type { SkillCatalogueEntry } from '../store/koma'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

let deleteSeq = 0

type Props = { skills: SkillCatalogueEntry[]; onClose: () => void }

export function SkillDeleteConfirm({ skills, onClose }: Props) {
  const req = useKoma((s) => s.req)
  const epoch = useKoma((s) => s.skillSessionEpoch)
  const opResults = useKoma((s) => s.skillOpResults)
  const registerSkillDelete = useKoma((s) => s.registerSkillDelete)
  const [requestId, setRequestId] = useState<string | null>(null)
  const restoreFocus = useRef<HTMLElement | null>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const closeRef = useRef(onClose)
  const busyRef = useRef(false)
  const ownResult = requestId ? opResults[requestId] : null
  const result = ownResult?.operation === 'delete' && ownResult.tabId === 'delete-dialog' ? ownResult : null
  closeRef.current = onClose
  busyRef.current = Boolean(requestId && !result)

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

  const confirm = () => {
    if (!skills.length || epoch !== useKoma.getState().skillSessionEpoch) return
    const id = `delete-${++deleteSeq}`
    registerSkillDelete(id, epoch, skills)
    setRequestId(id)
    req({
      r: 'DeleteSkills',
      items: skills.map((skill) => ({ skillId: skill.skillId, generation: skill.generation, name: skill.name })),
      requestId: id,
      sessionEpoch: epoch,
      tabId: 'delete-dialog',
    })
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-3" role="dialog" aria-modal="true" aria-labelledby="delete-skills-title">
      <div className="w-full max-w-xl rounded border border-koma-border bg-koma-panel shadow-xl">
        <div className="border-b border-koma-border px-4 py-3">
          <h2 id="delete-skills-title" className="text-[13px] font-semibold">Delete {skills.length} skill{skills.length === 1 ? '' : 's'}?</h2>
          <p className="mt-0.5 text-[11px] text-red-400">This permanently removes exactly the owned source paths below.</p>
        </div>
        <div className="max-h-[55vh] space-y-2 overflow-auto px-4 py-3">
          {skills.map((skill, index) => {
            const candidate = result?.outcomes[index]
            const outcome = candidate?.name === skill.name ? candidate : null
            return <div key={skill.skillId} className="rounded border border-koma-border px-2 py-1.5 text-[11px]">
              <div className="flex justify-between gap-2"><strong>{skill.name}</strong><span className="uppercase opacity-50">{skill.scope}</span></div>
              <div className="mt-0.5 break-all font-mono text-[10px] opacity-55">{skill.sourcePath}</div>
              {outcome && <div className={`mt-1 text-[10px] ${outcome.status === 'success' ? 'text-green-400' : 'text-red-400'}`}>{outcome.status === 'success' ? 'Deleted' : outcome.error ?? outcome.status}</div>}
            </div>
          })}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-koma-border px-4 py-3">
          <button ref={cancelRef} type="button" onClick={onClose} disabled={Boolean(requestId) && !result} className="rounded px-3 py-1.5 text-[11px] opacity-65 hover:bg-koma-hover disabled:opacity-30">{result ? 'Close' : 'Cancel'}</button>
          {!result && <button type="button" onClick={confirm} disabled={Boolean(requestId)} className="flex items-center gap-1.5 rounded bg-red-500/20 px-3 py-1.5 text-[11px] text-red-300 disabled:opacity-45">{requestId && <BrailleSpinner size={12} />} Delete forever</button>}
        </div>
      </div>
    </div>
  )
}
