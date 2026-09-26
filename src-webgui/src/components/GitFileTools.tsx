import { CodePane } from './GitCodePane'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowDown, ArrowUp, Check, Minus, Plus, RefreshCw, Save } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { GitToolTab } from '../lib/gitWorkbench'
import { Actions, Button, Confirm, Note, openCommit, openGitTool, setGitDirty, type GitWork } from './gitWorkbenchShared'
import DiffTab from './DiffTab'

type ImageData = { mime: string; size: number; base64: string }
type DiffData = { token: string; original: string; modified: string; partial: boolean; image: boolean; originalImage?: ImageData; modifiedImage?: ImageData; hunks: { header: string; oldStart: number; rows: { id: number; kind: string; text: string }[] }[] }
function ImageSide({ data, label, fit }: { data?: ImageData; label: string; fit: boolean }) {
  const [dimensions, setDimensions] = useState('')
  const [failed, setFailed] = useState(false)
  useEffect(() => { setDimensions(''); setFailed(false) }, [data])
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
    <div className="border-b border-koma-border px-3 py-1.5 text-[11px] text-koma-dim">{label}{data && ` · ${(data.size / 1024).toFixed(1)} KiB ${dimensions}`}</div>
    <div className="flex-1 overflow-auto p-4">
      {!data ? <Note>File absent</Note> : failed ? <Note>This image could not be decoded.</Note> : <img alt={label} src={`data:${data.mime};base64,${data.base64}`} className={fit ? 'mx-auto max-h-full max-w-full object-contain' : 'max-w-none'} onError={() => setFailed(true)} onLoad={e => setDimensions(`· ${e.currentTarget.naturalWidth} × ${e.currentTarget.naturalHeight}`)} />}
    </div>
  </div>
}
export function GitDiffView({ tab, work }: { tab: GitToolTab; work: GitWork }) {
  const [data, setData] = useState<DiffData>()
  const [selection, setSelection] = useState<number[]>([])
  const [split, setSplit] = useState(false)
  const [fit, setFit] = useState(true)
  const { run, busy, active } = work
  const reload = useCallback(async () => {
    const result = await run<DiffData>({ kind: 'diff', path: tab.path!, staged: !!tab.staged, commit: tab.commit, oldPath: tab.oldPath })
    if (result) { setData(result); setSelection([]) }
  }, [run, tab.path, tab.staged, tab.commit, tab.oldPath])
  useEffect(() => { void reload() }, [reload])
  const change = async (lines: number[]) => {
    if (!data) return
    if (await run({ kind: 'stageLines', path: tab.path!, staged: !!tab.staged, token: data.token, lines })) await reload()
  }
  const before = tab.commit ? 'Parent' : 'Index'
  const after = tab.commit ? tab.commit.slice(0, 8) : tab.staged ? 'HEAD · unstage target' : 'Working tree'
  const verb = tab.staged ? 'Unstage' : 'Stage'
  return <>
    <Actions>
      <Button disabled={busy || !active} onClick={() => void reload()} title="Refresh diff"><RefreshCw size={12} />Refresh</Button>
      <span className="mr-auto text-[11px] text-koma-dim">{before} → {after}</span>
      {data?.image ? <Button onClick={() => setFit(v => !v)}>{fit ? 'Actual size' : 'Fit'}</Button> : <Button onClick={() => setSplit(v => !v)}>{split ? 'Hunks' : 'Side by side'}</Button>}
      {!tab.commit && <Button disabled={!active} onClick={() => openGitTool('blame', { root: tab.root, path: tab.path })}>Blame HEAD</Button>}
      {data?.partial && <Button disabled={busy || !active || !selection.length} onClick={() => void change(selection)}>{tab.staged ? <Minus size={12} /> : <Plus size={12} />}{verb} selected ({selection.length})</Button>}
    </Actions>
    {!data ? <Note>{busy ? 'Loading diff…' : 'Refresh to load this diff.'}</Note> : data.image ? <div className="flex min-h-0 flex-1 divide-x divide-koma-border"><ImageSide data={data.originalImage} label={before} fit={fit} /><ImageSide data={data.modifiedImage} label={after} fit={fit} /></div> : <>
      {!data.partial && !tab.commit && <Note>Use the file actions in Source Control for this change. Partial staging is unavailable for renames, mode changes and non-text files.</Note>}
      {split || !data.partial ? <div className="min-h-0 flex-1"><DiffTab tab={{ id: tab.id, kind: 'diff', path: tab.path!, title: tab.title, loading: false, diff: { origin: 'git', original: data.original, modified: data.modified, binary: false, error: null } }} /></div> : <div className="min-h-0 flex-1 overflow-auto font-mono text-[12px]">
        {data.hunks.length === 0 && <Note>No text changes.</Note>}
        {data.hunks.map((h, i) => <div key={i} className="mb-3">
          <div className="sticky top-0 flex items-center gap-2 border-y border-koma-border bg-koma-panel px-3 py-1 text-koma-dim"><span className="min-w-0 flex-1 truncate">{h.header}</span><Button disabled={busy || !active} onClick={() => void change(h.rows.filter(r => r.kind !== ' ').map(r => r.id))}>{verb} hunk</Button></div>
          {h.rows.map(r => <label key={r.id} className={`flex min-w-max items-start gap-2 px-3 leading-5 ${r.kind === '+' ? 'bg-koma-success/10 text-koma-success' : r.kind === '-' ? 'bg-koma-error/10 text-koma-error' : 'text-koma-dim'}`}>
            {r.kind !== ' ' ? <input type="checkbox" aria-label={`Select changed line ${r.id}`} disabled={busy || !active} checked={selection.includes(r.id)} onChange={e => setSelection(s => e.target.checked ? [...s, r.id] : s.filter(id => id !== r.id))} className="mt-1 accent-koma-accent" /> : <span className="w-[13px]" />}
            <span className="w-3 select-none">{r.kind}</span><span className="whitespace-pre">{r.text.replace(/\r?\n$/, '') || ' '}</span>
          </label>)}
        </div>)}
      </div>}
    </>}
  </>
}

type ConflictData = { token: string; current: string; incoming: string; result: string; currentExists: boolean; incomingExists: boolean; binary: boolean }
function conflictBlocks(text: string) {
  const pattern = /^<<<<<<<[^\r\n]*\r?\n([\s\S]*?)^=======[\t ]*\r?\n([\s\S]*?)^>>>>>>>[^\r\n]*(?:\r?\n|$)/gm
  return [...text.matchAll(pattern)].map(m => ({ start: m.index!, end: m.index! + m[0].length,
    current: m[1].replace(/^\|{7}[^\r\n]*\r?\n[\s\S]*$/m, ''), incoming: m[2], line: text.slice(0, m.index).split('\n').length }))
}
export function GitConflictView({ tab, work }: { tab: GitToolTab; work: GitWork }) {
  const [data, setData] = useState<ConflictData>()
  const [result, setResult] = useState('')
  const [selected, setSelected] = useState(0)
  const [reloadConfirm, setReloadConfirm] = useState(false)
  const [choice, setChoice] = useState<string | null>(null)
  const [resolved, setResolved] = useState(false)
  const rebase = useKoma(s => s.git.inProgress === 'rebase')
  const { run, busy, active } = work
  const dirty = !!data && result !== data.result && !resolved
  useEffect(() => { setGitDirty(tab.id, dirty) }, [tab.id, dirty])
  const reload = useCallback(async () => {
    const next = await run<ConflictData>({ kind: 'conflict', path: tab.path! })
    if (next) { setData(next); setResult(next.result); setSelected(0); setResolved(false) }
    setReloadConfirm(false)
  }, [run, tab.path])
  useEffect(() => { void reload() }, [reload])
  const blocks = useMemo(() => conflictBlocks(result), [result])
  const index = Math.min(selected, Math.max(0, blocks.length - 1))
  const accept = (side: 'current' | 'incoming' | 'both') => {
    const b = blocks[index]
    if (!b) return
    setResult(result.slice(0, b.start) + (side === 'both' ? b.current + b.incoming : b[side]) + result.slice(b.end))
  }
  const save = async (stage: boolean, selectedChoice?: string) => {
    if (!data) return
    const next = await run<ConflictData>({ kind: 'resolve', path: tab.path!, token: data.token, content: selectedChoice ? undefined : result, choice: selectedChoice, stage })
    if (next) {
      setChoice(null)
      if (stage) { setResolved(true); setGitDirty(tab.id, false) }
      else { setData(next); setResult(next.result) }
    }
  }
  const currentLabel = rebase ? 'Current · rebased destination (ours)' : 'Current (ours)'
  const incomingLabel = rebase ? 'Incoming · commit being replayed (theirs)' : 'Incoming (theirs)'
  return <>
    <Actions>
      <Button disabled={busy || !active} onClick={() => dirty ? setReloadConfirm(true) : void reload()}><RefreshCw size={12} />Reload</Button>
      <span className="mr-auto text-[11px] text-koma-dim">{resolved ? 'Resolved and staged' : `${blocks.length} conflict(s)${dirty ? ' · unsaved' : ''}`}</span>
      {!data?.binary && !resolved && <><Button disabled={busy || !active || !dirty} onClick={() => void save(false)}><Save size={12} />Save result</Button><Button disabled={busy || !active || !data || blocks.length > 0} onClick={() => void save(true)}><Check size={12} />Mark resolved</Button></>}
    </Actions>
    {reloadConfirm && <Confirm message="Discard unsaved resolution and reload the file?" onConfirm={() => void reload()} onCancel={() => setReloadConfirm(false)} busy={busy} />}
    {choice && <Confirm message={`Use ${choice === 'delete' ? 'deletion' : choice + ' version'} for the entire file and stage it?`} onConfirm={() => void save(true, choice)} onCancel={() => setChoice(null)} busy={busy} />}
    {!data ? <Note>Loading conflict…</Note> : resolved ? <Note>File staged. Continue the operation from Source Control once every conflict is resolved.</Note> : <>
      {(data.binary || !data.currentExists || !data.incomingExists) && <>
        <Note>{data.binary ? 'Binary or large file: choose a whole version.' : 'This conflict includes a deletion. Keep a version, delete the file, or edit the result below.'}</Note>
        <Actions><Button disabled={busy || !active || !data.currentExists} onClick={() => setChoice('current')}>Keep {currentLabel}</Button><Button disabled={busy || !active || !data.incomingExists} onClick={() => setChoice('incoming')}>Keep {incomingLabel}</Button><Button disabled={busy || !active} onClick={() => setChoice('delete')}>Delete file</Button></Actions>
      </>}
      {!data.binary && <>
        <div className="grid min-h-0 flex-1 grid-cols-2 divide-x divide-koma-border border-y border-koma-border">
          {[{ label: currentLabel, text: data.current }, { label: incomingLabel, text: data.incoming }].map(s => <div key={s.label} className="flex min-h-0 min-w-0 flex-col"><div className="bg-koma-panel px-3 py-1 text-[11px] text-koma-dim">{s.label}</div><div className="min-h-0 flex-1"><CodePane value={s.text} path={tab.path} tabId={tab.id} /></div></div>)}
        </div>
        <Actions><span className="mr-auto text-[11px] text-koma-dim">Result · {blocks.length ? `${index + 1}/${blocks.length}` : 'no conflict blocks'}</span>
          <Button disabled={!blocks.length} title="Previous conflict" onClick={() => setSelected((index + blocks.length - 1) % blocks.length)}><ArrowUp size={12} /></Button>
          <Button disabled={!blocks.length} title="Next conflict" onClick={() => setSelected((index + 1) % blocks.length)}><ArrowDown size={12} /></Button>
          {(['current', 'incoming', 'both'] as const).map(s => <Button key={s} disabled={busy || !active || !blocks.length} onClick={() => accept(s)}>Accept {s}</Button>)}
        </Actions>
        <div className="min-h-0 flex-1 border-t border-koma-border"><CodePane value={result} onChange={setResult} readOnly={busy || !active} path={tab.path} tabId={tab.id} revealLine={blocks[index]?.line} /></div>
      </>}
    </>}
  </>
}

type BlameData = { head: string; workingTreeDiffers: boolean; rows: { oid: string; author: string; time: string; summary: string; text: string }[] }
export function GitBlameView({ tab, work }: { tab: GitToolTab; work: GitWork }) {
  const [data, setData] = useState<BlameData>()
  const [limit, setLimit] = useState(500)
  const { run, busy, active } = work
  const reload = useCallback(async () => { const next = await run<BlameData>({ kind: 'blame', path: tab.path! }); if (next) { setData(next); setLimit(500) } }, [run, tab.path])
  useEffect(() => { void reload() }, [reload])
  return <><Actions><Button disabled={busy || !active} onClick={() => void reload()}><RefreshCw size={12} />Refresh</Button><span className="text-[11px] text-koma-dim">HEAD {data?.head.slice(0, 8)} · read only</span></Actions>
    {data?.workingTreeDiffers && <Note>The working file differs from HEAD. These line numbers refer to the committed version.</Note>}
    <div className="min-h-0 flex-1 overflow-auto font-mono text-[12px]">
      {data?.rows.slice(0, limit).map((r, i) => <div key={i} className="flex min-w-max items-center border-b border-koma-border/30 leading-6 hover:bg-koma-hover">
        <button disabled={!active} onClick={() => openCommit(r.oid)} title={`${r.summary}\n${new Date(Number(r.time) * 1000).toLocaleString()}`} className="flex w-64 flex-none gap-2 truncate px-3 text-left text-koma-dim hover:text-koma-accent"><span>{r.oid.slice(0, 8)}</span><span className="truncate">{r.author}</span></button>
        <span className="w-12 flex-none select-none text-right text-koma-dim">{i + 1}</span><span className="whitespace-pre px-3 text-koma-fg">{r.text || ' '}</span>
      </div>)}
      {data && data.rows.length > limit && <Actions><Button onClick={() => setLimit(n => n + 500)}>Show next 500 lines</Button></Actions>}
    </div></>
}
