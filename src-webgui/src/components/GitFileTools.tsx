import { CodePane, type CodePaneScroll } from './GitCodePane'
import {
  BLAME_LANE_WIDTH,
  BLAME_LINE_HEIGHT,
  blameBlocks,
  blameLaneColor,
  relativeBlameTime,
  visibleBlameBlocks,
  type BlameBlock,
  type BlameRow,
} from '../lib/blameBlocks'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowDown,
  ArrowUp,
  Check,
  Minus,
  Plus,
  RefreshCw,
  Save,
} from 'lucide-react'
import { useKoma } from '../store/koma'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'
import type { GitToolTab } from '../lib/gitWorkbench'
import {
  Actions,
  Button,
  Confirm,
  Note,
  openCommit,
  openGitTool,
  setGitDirty,
  type GitWork,
} from './gitWorkbenchShared'
import DiffTab from './DiffTab'

type ImageData = { mime: string; size: number; base64: string }
type DiffData = {
  token: string
  original: string
  modified: string
  partial: boolean
  image: boolean
  originalImage?: ImageData
  modifiedImage?: ImageData
  hunks: {
    header: string
    oldStart: number
    rows: { id: number; kind: string; text: string }[]
  }[]
}
function ImageSide({
  data,
  label,
  fit,
}: {
  data?: ImageData
  label: string
  fit: boolean
}) {
  const [dimensions, setDimensions] = useState('')
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    setDimensions('')
    setFailed(false)
  }, [data])
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div className="border-b border-koma-border px-3 py-1.5 text-[11px] text-koma-dim">
        {label}
        {data && ` · ${(data.size / 1024).toFixed(1)} KiB ${dimensions}`}
      </div>
      <div className="flex-1 overflow-auto p-4">
        {!data ? (
          <Note>File absent</Note>
        ) : failed ? (
          <Note>This image could not be decoded.</Note>
        ) : (
          <img
            alt={label}
            src={`data:${data.mime};base64,${data.base64}`}
            className={
              fit
                ? 'mx-auto max-h-full max-w-full object-contain'
                : 'max-w-none'
            }
            onError={() => setFailed(true)}
            onLoad={(e) =>
              setDimensions(
                `· ${e.currentTarget.naturalWidth} × ${e.currentTarget.naturalHeight}`,
              )
            }
          />
        )}
      </div>
    </div>
  )
}
export function GitDiffView({ tab, work }: { tab: GitToolTab; work: GitWork }) {
  const [data, setData] = useState<DiffData>()
  const [selection, setSelection] = useState<number[]>([])
  const [split, setSplit] = useState(false)
  const [fit, setFit] = useState(true)
  const request = useRef(0)
  const token = useRef<string | undefined>(undefined)
  const visible = useKoma(s => isTabVisible(normalizeGroups(s.ui), tab.id))
  const status = useKoma(s => !tab.commit && s.git.root === tab.root ? s.git : null)
  const { run, busy, active } = work
  const reload = useCallback(async () => {
    const ticket = ++request.current
    const result = await run<DiffData>({
      kind: 'diff',
      path: tab.path!,
      staged: !!tab.staged,
      commit: tab.commit,
      oldPath: tab.oldPath,
    })
    if (ticket !== request.current) return
    if (result) {
      setData(result)
      if (token.current !== result.token) setSelection([])
      token.current = result.token
    } else {
      setData(undefined)
      setSelection([])
      token.current = undefined
    }
  }, [run, tab.path, tab.staged, tab.commit, tab.oldPath])
  useLayoutEffect(() => {
    setData(undefined)
    setSelection([])
    token.current = undefined
    return () => { request.current++ }
  }, [reload])
  useEffect(() => {
    if (!active || !visible) return
    void reload()
    // A status refresh or tab switch can overlap a slow remote response.
    return () => { request.current++ }
  }, [reload, active, visible, status])
  const change = async (lines: number[]) => {
    if (!data) return
    if (
      await run({
        kind: 'stageLines',
        path: tab.path!,
        staged: !!tab.staged,
        token: data.token,
        lines,
      })
    )
      await reload()
  }
  const reversed =
    !!tab.staged && !tab.commit && (!!data?.image || split || !data?.partial)
  const before = tab.commit ? 'Parent' : reversed ? 'HEAD' : 'Index'
  const after = tab.commit
    ? tab.commit.slice(0, 8)
    : reversed
      ? 'Index'
      : tab.staged
        ? 'HEAD · unstage target'
        : 'Working tree'
  const verb = tab.staged ? 'Unstage' : 'Stage'
  return (
    <>
      <Actions>
        <Button
          disabled={busy || !active}
          onClick={() => void reload()}
          title="Refresh diff"
        >
          <RefreshCw size={12} />
          Refresh
        </Button>
        <span className="mr-auto text-[11px] text-koma-dim">
          {before} → {after}
        </span>
        {data?.image ? (
          <Button onClick={() => setFit((v) => !v)}>
            {fit ? 'Actual size' : 'Fit'}
          </Button>
        ) : (
          <Button onClick={() => setSplit((v) => !v)}>
            {split ? 'Hunks' : 'Side by side'}
          </Button>
        )}
        {!tab.commit && (
          <Button
            disabled={!active}
            onClick={() =>
              openGitTool('blame', { root: tab.root, path: tab.path })
            }
          >
            Blame HEAD
          </Button>
        )}
        {data?.partial && (
          <Button
            disabled={busy || !active || !selection.length}
            onClick={() => void change(selection)}
          >
            {tab.staged ? <Minus size={12} /> : <Plus size={12} />}
            {verb} selected ({selection.length})
          </Button>
        )}
      </Actions>
      {!data ? (
        active && !work.error ? <Note>Loading diff…</Note> : null
      ) : data.image ? (
        <div className="flex min-h-0 flex-1 divide-x divide-koma-border">
          <ImageSide
            data={reversed ? data.modifiedImage : data.originalImage}
            label={before}
            fit={fit}
          />
          <ImageSide
            data={reversed ? data.originalImage : data.modifiedImage}
            label={after}
            fit={fit}
          />
        </div>
      ) : (
        <>
          {!data.partial && !tab.commit && (
            <Note>
              Use the file actions in Source Control for this change. Partial
              staging is unavailable for renames, mode changes, filtered files
              and non-text files.
            </Note>
          )}
          {split || !data.partial ? (
            <div className="min-h-0 flex-1">
              <DiffTab
                tab={{
                  id: tab.id,
                  kind: 'diff',
                  path: tab.path!,
                  title: tab.title,
                  loading: false,
                  diff: {
                    origin: 'git',
                    original: reversed ? data.modified : data.original,
                    modified: reversed ? data.original : data.modified,
                    binary: false,
                    error: null,
                  },
                }}
              />
            </div>
          ) : (
            <div className="min-h-0 flex-1 overflow-auto font-mono text-[12px]">
              {data.hunks.length === 0 && <Note>No text changes.</Note>}
              {data.hunks.map((h, i) => (
                <div key={i} className="mb-3">
                  <div className="sticky top-0 flex items-center gap-2 border-y border-koma-border bg-koma-panel px-3 py-1 text-koma-dim">
                    <span className="min-w-0 flex-1 truncate">{h.header}</span>
                    <Button
                      disabled={busy || !active}
                      onClick={() =>
                        void change(
                          h.rows.filter((r) => r.kind !== ' ').map((r) => r.id),
                        )
                      }
                    >
                      {verb} hunk
                    </Button>
                  </div>
                  {h.rows.map((r) => (
                    <label
                      key={r.id}
                      className={`flex min-w-max items-start gap-2 px-3 leading-5 ${r.kind === '+' ? 'bg-koma-success/10 text-koma-success' : r.kind === '-' ? 'bg-koma-error/10 text-koma-error' : 'text-koma-dim'}`}
                    >
                      {r.kind !== ' ' ? (
                        <input
                          type="checkbox"
                          aria-label={`Select changed line ${r.id}`}
                          disabled={busy || !active}
                          checked={selection.includes(r.id)}
                          onChange={(e) =>
                            setSelection((s) =>
                              e.target.checked
                                ? [...s, r.id]
                                : s.filter((id) => id !== r.id),
                            )
                          }
                          className="mt-1 accent-koma-accent"
                        />
                      ) : (
                        <span className="w-[13px]" />
                      )}
                      <span className="w-3 select-none">{r.kind}</span>
                      <span className="whitespace-pre">
                        {r.text.replace(/\r?\n$/, '') || ' '}
                      </span>
                    </label>
                  ))}
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </>
  )
}

type ConflictData = {
  token: string
  current: string
  incoming: string
  result: string
  currentExists: boolean
  incomingExists: boolean
  binary: boolean
}
function conflictBlocks(text: string) {
  const pattern =
    /^<<<<<<<[^\r\n]*\r?\n([\s\S]*?)^=======[\t ]*\r?\n([\s\S]*?)^>>>>>>>[^\r\n]*(?:\r?\n|$)/gm
  return [...text.matchAll(pattern)].map((m) => ({
    start: m.index!,
    end: m.index! + m[0].length,
    current: m[1].replace(/^\|{7}[^\r\n]*\r?\n[\s\S]*$/m, ''),
    incoming: m[2],
    line: text.slice(0, m.index).split('\n').length,
  }))
}
export function GitConflictView({
  tab,
  work,
}: {
  tab: GitToolTab
  work: GitWork
}) {
  const [data, setData] = useState<ConflictData>()
  const [result, setResult] = useState('')
  const [selected, setSelected] = useState(0)
  const [revealTick, setRevealTick] = useState(0)
  const [reloadConfirm, setReloadConfirm] = useState(false)
  const [choice, setChoice] = useState<string | null>(null)
  const [resolved, setResolved] = useState(false)
  const rebase = useKoma((s) => s.git.inProgress === 'rebase')
  const { run, busy, active } = work
  const dirty = !!data && result !== data.result && !resolved
  useEffect(() => {
    setGitDirty(tab.id, dirty)
  }, [tab.id, dirty])
  const reload = useCallback(async () => {
    const next = await run<ConflictData>({ kind: 'conflict', path: tab.path! })
    if (next) {
      setData(next)
      setResult(next.result)
      setSelected(0)
      setResolved(false)
    }
    setReloadConfirm(false)
  }, [run, tab.path])
  useEffect(() => {
    void reload()
  }, [reload])
  const blocks = useMemo(() => conflictBlocks(result), [result])
  const index = Math.min(selected, Math.max(0, blocks.length - 1))
  const accept = (side: 'current' | 'incoming' | 'both') => {
    const b = blocks[index]
    if (!b) return
    setRevealTick((t) => t + 1)
    setResult(
      result.slice(0, b.start) +
        (side === 'both' ? b.current + b.incoming : b[side]) +
        result.slice(b.end),
    )
  }
  const save = async (stage: boolean, selectedChoice?: string) => {
    if (!data) return
    const next = await run<ConflictData>({
      kind: 'resolve',
      path: tab.path!,
      token: data.token,
      content: selectedChoice ? undefined : result,
      choice: selectedChoice,
      stage,
    })
    if (next) {
      setChoice(null)
      if (stage) {
        setResolved(true)
        setGitDirty(tab.id, false)
      } else {
        setData(next)
        setResult(next.result)
      }
    }
  }
  const currentLabel = rebase
    ? 'Current · rebased destination (ours)'
    : 'Current (ours)'
  const incomingLabel = rebase
    ? 'Incoming · commit being replayed (theirs)'
    : 'Incoming (theirs)'
  return (
    <>
      <Actions>
        <Button
          disabled={busy || !active}
          onClick={() => (dirty ? setReloadConfirm(true) : void reload())}
        >
          <RefreshCw size={12} />
          Reload
        </Button>
        <span className="mr-auto text-[11px] text-koma-dim">
          {resolved
            ? 'Resolved and staged'
            : `${blocks.length} conflict(s)${dirty ? ' · unsaved' : ''}`}
        </span>
        {!data?.binary && !resolved && (
          <>
            <Button
              disabled={busy || !active || !dirty}
              onClick={() => void save(false)}
            >
              <Save size={12} />
              Save result
            </Button>
            <Button
              disabled={busy || !active || !data || blocks.length > 0}
              onClick={() => void save(true)}
            >
              <Check size={12} />
              Mark resolved
            </Button>
          </>
        )}
      </Actions>
      {reloadConfirm && (
        <Confirm
          message="Discard unsaved resolution and reload the file?"
          onConfirm={() => void reload()}
          onCancel={() => setReloadConfirm(false)}
          busy={busy}
        />
      )}
      {choice && (
        <Confirm
          message={`Use ${choice === 'delete' ? 'deletion' : choice + ' version'} for the entire file and stage it?`}
          onConfirm={() => void save(true, choice)}
          onCancel={() => setChoice(null)}
          busy={busy}
        />
      )}
      {!data ? (
        <Note>Loading conflict…</Note>
      ) : resolved ? (
        <Note>
          File staged. Continue the operation from Source Control once every
          conflict is resolved.
        </Note>
      ) : (
        <>
          {(data.binary || !data.currentExists || !data.incomingExists) && (
            <>
              <Note>
                {data.binary
                  ? 'Binary or large file: choose a whole version.'
                  : 'This conflict includes a deletion. Keep a version, delete the file, or edit the result below.'}
              </Note>
              <Actions>
                <Button
                  disabled={busy || !active || !data.currentExists}
                  onClick={() => setChoice('current')}
                >
                  Keep {currentLabel}
                </Button>
                <Button
                  disabled={busy || !active || !data.incomingExists}
                  onClick={() => setChoice('incoming')}
                >
                  Keep {incomingLabel}
                </Button>
                <Button
                  disabled={busy || !active}
                  onClick={() => setChoice('delete')}
                >
                  Delete file
                </Button>
              </Actions>
            </>
          )}
          {!data.binary && (
            <>
              <div className="grid min-h-0 flex-1 grid-cols-2 divide-x divide-koma-border border-y border-koma-border">
                {[
                  { label: currentLabel, text: data.current },
                  { label: incomingLabel, text: data.incoming },
                ].map((s) => (
                  <div key={s.label} className="flex min-h-0 min-w-0 flex-col">
                    <div className="bg-koma-panel px-3 py-1 text-[11px] text-koma-dim">
                      {s.label}
                    </div>
                    <div className="min-h-0 flex-1">
                      <CodePane value={s.text} path={tab.path} tabId={tab.id} />
                    </div>
                  </div>
                ))}
              </div>
              <Actions>
                <span className="mr-auto text-[11px] text-koma-dim">
                  Result ·{' '}
                  {blocks.length
                    ? `${index + 1}/${blocks.length}`
                    : 'no conflict blocks'}
                </span>
                <Button
                  disabled={!blocks.length}
                  title="Previous conflict"
                  onClick={() => {
                    setSelected((index + blocks.length - 1) % blocks.length)
                    setRevealTick((t) => t + 1)
                  }}
                >
                  <ArrowUp size={12} />
                </Button>
                <Button
                  disabled={!blocks.length}
                  title="Next conflict"
                  onClick={() => {
                    setSelected((index + 1) % blocks.length)
                    setRevealTick((t) => t + 1)
                  }}
                >
                  <ArrowDown size={12} />
                </Button>
                {(['current', 'incoming', 'both'] as const).map((s) => (
                  <Button
                    key={s}
                    disabled={busy || !active || !blocks.length}
                    onClick={() => accept(s)}
                  >
                    Accept {s}
                  </Button>
                ))}
              </Actions>
              <div className="min-h-0 flex-1 border-t border-koma-border">
                <CodePane
                  value={result}
                  onChange={setResult}
                  readOnly={busy || !active}
                  path={tab.path}
                  tabId={tab.id}
                  revealLine={blocks[index]?.line}
                  revealTick={revealTick}
                />
              </div>
            </>
          )}
        </>
      )}
    </>
  )
}

type BlameData = {
  head: string
  workingTreeDiffers: boolean
  rows: BlameRow[]
}

function blameWhen(time: string): string {
  const n = Number(time)
  if (!Number.isFinite(n) || n <= 0) return ''
  return new Date(n * 1000).toLocaleString()
}

function BlameCard({
  block,
  anchorTop,
  viewHeight,
  active,
  onOpen,
  onEnter,
  onLeave,
}: {
  block: BlameBlock
  anchorTop: number
  viewHeight: number
  active: boolean
  onOpen: () => void
  onEnter: () => void
  onLeave: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [top, setTop] = useState(anchorTop)
  const when = blameWhen(block.time)
  const relative = relativeBlameTime(block.time)
  useLayoutEffect(() => {
    const height = ref.current?.offsetHeight ?? 0
    const maxTop = Math.max(8, viewHeight - height - 8)
    setTop(Math.min(Math.max(8, anchorTop), maxTop))
  }, [anchorTop, viewHeight, block.body, block.summary, block.oid])
  return (
    <div
      ref={ref}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      className="absolute z-30 w-72 max-w-[calc(100%-184px)] overflow-auto rounded border border-koma-border bg-koma-panel p-2.5 text-[12px] text-koma-fg shadow-lg"
      style={{
        left: BLAME_LANE_WIDTH + 8,
        top,
        maxHeight: Math.max(96, viewHeight - 16),
      }}
    >
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 truncate font-medium">{block.author}</span>
        <span className="shrink-0 text-[11px] text-koma-dim">
          {relative}
          {when ? ` · ${when}` : ''}
        </span>
      </div>
      <p className="mt-1">{block.summary || block.oid.slice(0, 8)}</p>
      {block.body && (
        <p className="mt-1 line-clamp-4 whitespace-pre-wrap text-[11px] text-koma-dim">
          {block.body}
        </p>
      )}
      <button
        type="button"
        disabled={!active}
        onClick={onOpen}
        className="mt-2 font-mono text-[11px] text-koma-accent hover:underline disabled:opacity-35"
      >
        {block.oid.slice(0, 7)}
      </button>
    </div>
  )
}

export function GitBlameView({
  tab,
  work,
}: {
  tab: GitToolTab
  work: GitWork
}) {
  const [data, setData] = useState<BlameData>()
  const [scrollTop, setScrollTop] = useState(0)
  const [viewHeight, setViewHeight] = useState(0)
  const [lineHeight, setLineHeight] = useState(BLAME_LINE_HEIGHT)
  const [hover, setHover] = useState<number | null>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const laneRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<CodePaneScroll | null>(null)
  const desiredRef = useRef(0)
  const lineHeightRef = useRef(lineHeight)
  const closeTimer = useRef<number | null>(null)
  lineHeightRef.current = lineHeight
  const { run, busy, active } = work
  const reload = useCallback(async () => {
    const next = await run<BlameData>({ kind: 'blame', path: tab.path! })
    if (next) setData(next)
  }, [run, tab.path])
  useEffect(() => {
    void reload()
  }, [reload])
  const blocks = useMemo(() => blameBlocks(data?.rows ?? []), [data])
  const text = useMemo(
    () => (data?.rows ?? []).map((row) => row.text).join('\n'),
    [data],
  )
  const visible = useMemo(
    () => visibleBlameBlocks(blocks, scrollTop, viewHeight, lineHeight),
    [blocks, scrollTop, viewHeight, lineHeight],
  )
  const hovered = blocks.find((block) => block.start === hover) ?? null
  const cancelClose = useCallback(() => {
    if (closeTimer.current != null) window.clearTimeout(closeTimer.current)
    closeTimer.current = null
  }, [])
  const armClose = useCallback(() => {
    cancelClose()
    closeTimer.current = window.setTimeout(() => setHover(null), 100)
  }, [cancelClose])
  useEffect(() => () => cancelClose(), [cancelClose])
  useEffect(() => {
    setScrollTop(0)
    desiredRef.current = 0
    setHover(null)
  }, [tab.path])
  useLayoutEffect(() => {
    const frame = frameRef.current
    if (!frame) return
    const measure = () => setViewHeight(frame.clientHeight)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(frame)
    return () => observer.disconnect()
  }, [data])
  useEffect(() => {
    const lane = laneRef.current
    if (!lane) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const dy =
        event.deltaMode === WheelEvent.DOM_DELTA_LINE
          ? event.deltaY * lineHeightRef.current
          : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
            ? event.deltaY * lane.clientHeight
            : event.deltaY
      desiredRef.current = Math.max(0, desiredRef.current + dy)
      scrollRef.current?.setScrollTop(desiredRef.current)
    }
    lane.addEventListener('wheel', onWheel, { passive: false })
    return () => lane.removeEventListener('wheel', onWheel)
  }, [data])
  const onScroll = useCallback((next: number) => {
    desiredRef.current = next
    setScrollTop(next)
  }, [])
  const openHovered = (block: BlameBlock) => {
    if (active) openCommit(block.oid)
  }
  return (
    <>
      <Actions>
        <Button disabled={busy || !active} onClick={() => void reload()}>
          <RefreshCw size={12} />
          Refresh
        </Button>
        <span className="text-[11px] text-koma-dim">
          HEAD {data?.head.slice(0, 8)} · read only
        </span>
      </Actions>
      {data?.workingTreeDiffers && (
        <Note>
          The working file differs from HEAD. These line numbers refer to the
          committed version.
        </Note>
      )}
      <div
        ref={frameRef}
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
        <div
          ref={laneRef}
          className="relative shrink-0 overflow-hidden border-r border-koma-border"
          style={{ width: BLAME_LANE_WIDTH }}
        >
          {visible.map((block) => (
            <button
              key={block.start}
              type="button"
              tabIndex={-1}
              onMouseEnter={() => {
                cancelClose()
                setHover(block.start)
              }}
              onMouseLeave={armClose}
              onClick={() => openHovered(block)}
              className="absolute left-0 right-0 border-0 bg-transparent p-0 text-left hover:bg-koma-hover"
              style={{
                top: (block.start - 1) * lineHeight - scrollTop,
                height: (block.end - block.start + 1) * lineHeight,
                background:
                  hover === block.start ? 'var(--color-koma-hover)' : undefined,
              }}
            >
              <span
                className="absolute bottom-0 left-0 top-0 w-0.5"
                style={{ background: blameLaneColor(block.oid) }}
              />
              <span
                className="flex min-w-0 items-center gap-1.5 pl-2 pr-1.5"
                style={{ height: lineHeight }}
              >
                <span className="truncate text-[11px] text-koma-dim">
                  {block.summary || block.oid.slice(0, 8)}
                </span>
                <span className="shrink-0 text-[10px] text-koma-dim">
                  {relativeBlameTime(block.time)}
                </span>
              </span>
            </button>
          ))}
        </div>
        <div className="min-h-0 min-w-0 flex-1">
          {data && (
            <CodePane
              value={text}
              path={tab.path ?? ''}
              tabId={tab.id}
              readOnly
              lineHeight={BLAME_LINE_HEIGHT}
              folding={false}
              stickyScroll={false}
              wordWrap="off"
              onScroll={onScroll}
              onLineHeight={setLineHeight}
              scrollRef={scrollRef}
              highlight={
                hovered
                  ? { start: hovered.start, end: hovered.end }
                  : null
              }
            />
          )}
        </div>
        {hovered && (
          <BlameCard
            block={hovered}
            anchorTop={(hovered.start - 1) * lineHeight - scrollTop}
            viewHeight={viewHeight}
            active={active}
            onOpen={() => openHovered(hovered)}
            onEnter={cancelClose}
            onLeave={armClose}
          />
        )}
      </div>
    </>
  )
}
