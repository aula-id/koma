import { useRef, useState } from 'react'
import { Check } from 'lucide-react'
import { useKoma } from '../store/koma'
import { Confirm, useGitWork } from './gitWorkbenchShared'

export function GitCommitForm({ root, branchLabel }: { root: string; branchLabel: string }) {
  const draft = useKoma(s => s.commitDraft)
  const setDraft = useKoma(s => s.setCommitDraft)
  const staged = useKoma(s => s.git.staged.length)
  const conflicts = useKoma(s => s.git.conflicted.length)
  const inProgress = useKoma(s => s.git.inProgress)
  const gitCommit = useKoma(s => s.gitCommit)
  const work = useGitWork(root)
  const [head, setHead] = useState<string>()
  const [confirm, setConfirm] = useState(false)
  const previous = useRef('')
  const toggle = async () => {
    setConfirm(false)
    if (head) { setHead(undefined); setDraft(previous.current); return }
    const data = await work.run<{ head: string; message: string }>({ kind: 'commitMessage' })
    if (data) { previous.current = draft; setHead(data.head); setDraft(data.message.trimEnd()) }
  }
  const allowed = work.active && !work.busy && draft.trim().length > 0 && !conflicts && (head ? !inProgress : staged > 0)
  return <div className="flex flex-none flex-col gap-1.5 border-b border-koma-border px-3 py-2">
    <textarea value={draft} onChange={e => { setDraft(e.target.value); setConfirm(false) }} placeholder={`Message (${branchLabel})`} rows={2} disabled={work.busy} className="w-full resize-none rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-50 focus:outline-none focus:ring-1 focus:ring-koma-accent" />
    <label className="flex items-center gap-1.5 text-[11px] text-koma-dim"><input type="checkbox" checked={!!head} onChange={() => void toggle()} disabled={work.busy || !work.active || !!inProgress} />Amend last commit {head && <span className="font-mono">{head.slice(0, 8)}</span>}</label>
    {head && <span className="text-[10px] text-koma-dim">Uses staged changes only. Unstaged edits stay in the working tree.</span>}
    <button type="button" disabled={!allowed} onClick={() => head ? setConfirm(true) : gitCommit(draft)} className="flex items-center justify-center gap-1.5 rounded bg-koma-accent px-3 py-1.5 text-[12px] font-semibold text-koma-bg transition-opacity disabled:cursor-not-allowed disabled:opacity-35"><Check size={13} className="flex-none" />{head ? 'Amend commit' : 'Commit'}</button>
    {confirm && head && <Confirm message={`Replace commit ${head.slice(0, 8)}? This rewrites its identity, including when it has already been pushed.`} label="Amend" busy={!allowed} onCancel={() => setConfirm(false)} onConfirm={async () => {
      if (await work.run({ kind: 'amend', message: draft, head })) { setHead(undefined); setConfirm(false); setDraft('') }
    }} />}
    {work.error && <span role="alert" className="whitespace-pre-wrap break-words text-[11px] text-koma-error">{work.error}</span>}
  </div>
}
