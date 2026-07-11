import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import {
  GitBranch,
  ArrowUp,
  ArrowDown,
  FileText,
  Search,
  Plus,
  Minus,
  Undo2,
  Check,
  X,
  KeyRound,
  Loader2,
  RefreshCw,
  GitGraph,
  Maximize2,
} from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { BranchSwitcher } from '../BranchSwitcher'
import { GitGraphMini } from '../GitGraphMini'
import { ConflictBanner } from '../ConflictBanner'
import { Empty } from './helpers'
import { useKoma } from '../../store/koma'
import type { GitFileEntry } from '../../store/koma'

// git-porcelain status char -> badge tone. Mirrors ExplorePanel's FILE_STATUS
// idiom (added = good, modified = accent, deleted = error): A/? = new
// (good), M = touched (accent), D = removed (error), R/C = a rename/copy
// (warn — worth a second glance), U = unmerged/conflict (error, needs
// attention).
const STATUS_TONE: Record<string, string> = {
  A: 'text-koma-success',
  '?': 'text-koma-success',
  M: 'text-koma-accent',
  D: 'text-koma-error',
  R: 'text-koma-warn',
  C: 'text-koma-warn',
  U: 'text-koma-error',
}

function baseName(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

function dirName(path: string): string {
  const parts = path.split('/')
  parts.pop()
  return parts.join('/')
}

// A subtle row-hover action button (stage/unstage/discard) — invisible until
// the row is hovered/focused, mirroring VSCode's Source Control row actions.
// Always stops propagation so clicking it never also fires the row's own
// onClick (which opens the diff tab).
function RowAction({
  title,
  onClick,
  children,
}: {
  title: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition-opacity hover:bg-koma-hover group-hover:opacity-70 hover:!opacity-100 focus-visible:opacity-100"
    >
      {children}
    </button>
  )
}

// Small header-row action (Stage All / Unstage All / Discard All Changes) —
// shown on the AccordionSection header's hover-revealed `action` slot.
function HeaderAction({
  title,
  onClick,
  children,
}: {
  title: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
    >
      {children}
    </button>
  )
}

// A sync-toolbar button (Fetch/Pull/Push) — disabled + swaps its icon for a
// spinner while ITS OWN op (`busy`) is the one currently in flight; disabled
// (no spinner) while a DIFFERENT op is running (only one remote op runs at a
// time). `badge` renders a small trailing count (ahead/behind) when > 0.
function SyncButton({
  title,
  onClick,
  disabled,
  busy,
  badge,
  children,
}: {
  title: string
  onClick: () => void
  disabled: boolean
  busy: boolean
  badge?: number
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className="flex flex-none items-center gap-1 rounded px-1.5 py-1 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-35"
    >
      {busy ? <Loader2 size={13} className="flex-none animate-spin" /> : children}
      {typeof badge === 'number' && badge > 0 && (
        <span className="font-mono text-[11px] leading-none">{badge}</span>
      )}
    </button>
  )
}

// Full-row inline confirmation, REPLACING a row's normal content — the same
// idiom SessionRowConfirmStrip uses for kill/delete, kept local here (no
// session-specific coupling needed for a plain file-path discard).
function DiscardConfirmRow({
  label,
  onConfirm,
  onCancel,
}: {
  label: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      className="flex min-h-[30px] items-center justify-between gap-2 bg-koma-error/10 px-3 py-1 text-[12px] font-medium text-koma-error"
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="flex flex-none items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          aria-label="Confirm discard"
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 font-semibold opacity-90 hover:bg-koma-hover hover:opacity-100"
        >
          <Check size={12} className="flex-none" />
          discard
        </button>
        <button
          type="button"
          onClick={onCancel}
          aria-label="Cancel"
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <X size={12} className="flex-none" />
          cancel
        </button>
      </span>
    </div>
  )
}

// One GIT-panel file row: basename (+ dim parent dir), a rename shows
// `origPath -> path` instead, and a trailing status-char badge. Click opens
// the staged/unstaged Monaco diff for this path. `onStage`/`onUnstage`/
// `onDiscard` (each optional — a row only gets the actions that apply to its
// list) render subtle hover buttons that stopPropagation so they never also
// open the diff.
function FileRow({
  entry,
  onClick,
  onStage,
  onUnstage,
  onDiscard,
}: {
  entry: GitFileEntry
  onClick: () => void
  onStage?: () => void
  onUnstage?: () => void
  onDiscard?: () => void
}) {
  const tone = STATUS_TONE[entry.status] ?? 'text-koma-dim'
  const dir = dirName(entry.path)
  return (
    <div
      title={entry.origPath ? `${entry.origPath} -> ${entry.path}` : entry.path}
      onClick={onClick}
      className="group flex min-h-[30px] cursor-pointer items-center gap-1.5 px-3 py-1 hover:bg-koma-hover"
    >
      <FileText size={13} className="flex-none text-koma-fg opacity-45" />
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">
        {entry.origPath ? (
          <>
            <span className="opacity-60">{baseName(entry.origPath)}</span>
            <span className="opacity-40"> {'->'} </span>
            {baseName(entry.path)}
          </>
        ) : (
          <>
            {baseName(entry.path)}
            {dir && <span className="ml-1.5 text-koma-dim opacity-45">{dir}</span>}
          </>
        )}
      </span>
      {onDiscard && (
        <RowAction title="Discard changes" onClick={onDiscard}>
          <Undo2 size={13} />
        </RowAction>
      )}
      {onStage && (
        <RowAction title="Stage changes" onClick={onStage}>
          <Plus size={13} />
        </RowAction>
      )}
      {onUnstage && (
        <RowAction title="Unstage changes" onClick={onUnstage}>
          <Minus size={13} />
        </RowAction>
      )}
      <span className={`flex-none font-mono text-[11px] font-semibold ${tone}`}>{entry.status}</span>
    </div>
  )
}

// VSCode-style Source Control panel: a commit box, Staged Changes (unstage /
// commit), and Changes (stage / discard) — mirroring `git status
// --porcelain`'s staged/unstaged split. Discard is destructive, gated behind
// an inline per-row (or section-wide) confirm.
export function GitPanel() {
  const [open, setOpen] = useState({ staged: true, unstaged: true, conflicts: true, graph: true })
  const [filter, setFilter] = useState('')
  // The single armed discard confirm across BOTH the per-row buttons and the
  // section-wide "Discard All Changes" action — a sentinel string for the
  // latter (no real path collides with it). Local to the panel: an unmount
  // (switching sidebar views) naturally disarms it.
  const [armedDiscard, setArmedDiscard] = useState<string | null>(null)
  const DISCARD_ALL = '\0discard-all'

  const git = useKoma((s) => s.git)
  const keys = useKoma((s) => s.keys)
  const remoteBusy = useKoma((s) => s.remoteBusy)
  const commitDraft = useKoma((s) => s.commitDraft)
  const setCommitDraft = useKoma((s) => s.setCommitDraft)
  const refreshGitStatus = useKoma((s) => s.refreshGitStatus)
  const refreshKeys = useKoma((s) => s.refreshKeys)
  const openGitDiffTab = useKoma((s) => s.openGitDiffTab)
  const gitStage = useKoma((s) => s.gitStage)
  const gitUnstage = useKoma((s) => s.gitUnstage)
  const gitDiscard = useKoma((s) => s.gitDiscard)
  const gitCommit = useKoma((s) => s.gitCommit)
  const setGitKey = useKoma((s) => s.setGitKey)
  const gitFetch = useKoma((s) => s.gitFetch)
  const gitPull = useKoma((s) => s.gitPull)
  const gitPush = useKoma((s) => s.gitPush)
  const openGraphTab = useKoma((s) => s.openGraphTab)
  const sessionId = useKoma((s) => s.session.id)

  // Fetch fresh status on mount (panel opened/activated). GitPanel unmounts
  // when the sidebar switches to another view, so re-selecting "Source
  // Control" re-runs this — no separate "became active" plumbing needed. Also
  // (re)fetch the SSH vault list so the key picker below has options. Keyed on
  // `sessionId` too: the panel can stay mounted ACROSS a session switch (the
  // sidebar view state is unaffected), so this re-fires to load the NEW
  // session's repo instead of showing the old one's stale status.
  useEffect(() => {
    refreshGitStatus()
    refreshKeys()
  }, [refreshGitStatus, refreshKeys, sessionId])

  // `error` set OR no resolved root means the workdir isn't inside a git
  // repository at all — distinct from a detached-HEAD repo (still a real
  // repo, just no branch name), which renders normally below.
  if (git.error || !git.root) {
    return (
      <div className="flex h-full flex-col overflow-hidden">
        <Empty>{git.error ?? 'Not a git repository'}</Empty>
      </div>
    )
  }

  const q = filter.trim().toLowerCase()
  const staged = q ? git.staged.filter((e) => e.path.toLowerCase().includes(q)) : git.staged
  const unstaged = q ? git.unstaged.filter((e) => e.path.toLowerCase().includes(q)) : git.unstaged
  const conflicted = q ? git.conflicted.filter((e) => e.path.toLowerCase().includes(q)) : git.conflicted
  // A conflict (unmerged) file lands in NEITHER staged nor unstaged (see
  // compute_git_status's separate "u" record handling) — without folding
  // `conflicted` in here, an active conflict with otherwise-empty staged/
  // unstaged lists would wrongly render "No changes" and hide the Conflicts
  // section below.
  const clean = git.staged.length === 0 && git.unstaged.length === 0 && git.conflicted.length === 0
  const branchLabel = git.detached ? 'detached HEAD' : (git.branch ?? '(unknown)')
  const canCommit = commitDraft.trim().length > 0 && git.staged.length > 0

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <ConflictBanner />
      <div className="flex flex-none items-center gap-1.5 border-b border-koma-border px-3 py-2 font-mono text-[12px] text-koma-fg">
        <GitBranch size={13} className="flex-none opacity-60" />
        <span className="truncate">{branchLabel}</span>
        {(git.ahead ?? 0) > 0 && (
          <span className="flex flex-none items-center gap-0.5 text-koma-dim" title={`${git.ahead} commit(s) ahead`}>
            <ArrowUp size={11} />
            {git.ahead}
          </span>
        )}
        {(git.behind ?? 0) > 0 && (
          <span className="flex flex-none items-center gap-0.5 text-koma-dim" title={`${git.behind} commit(s) behind`}>
            <ArrowDown size={11} />
            {git.behind}
          </span>
        )}
        <BranchSwitcher variant="icon" />
        <span className="flex-1" />
        <button
          type="button"
          onClick={openGraphTab}
          title="Commit graph"
          aria-label="Commit graph"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <GitGraph size={13} />
        </button>
      </div>
      <div className="flex flex-none items-center gap-1 border-b border-koma-border px-3 py-1.5">
        <SyncButton
          title="Fetch"
          onClick={gitFetch}
          disabled={!!remoteBusy}
          busy={remoteBusy === 'fetch'}
        >
          <RefreshCw size={13} />
        </SyncButton>
        <SyncButton
          title={(git.behind ?? 0) > 0 ? `Pull (${git.behind} behind)` : 'Pull'}
          onClick={gitPull}
          disabled={!!remoteBusy}
          busy={remoteBusy === 'pull'}
          badge={git.behind ?? 0}
        >
          <ArrowDown size={13} />
        </SyncButton>
        <SyncButton
          title={(git.ahead ?? 0) > 0 ? `Push (${git.ahead} ahead)` : 'Push'}
          onClick={gitPush}
          disabled={!!remoteBusy}
          busy={remoteBusy === 'push'}
          badge={git.ahead ?? 0}
        >
          <ArrowUp size={13} />
        </SyncButton>
        <span className="flex-1" />
        <KeyRound size={12} className="flex-none text-koma-dim opacity-60" />
        <select
          value={git.keyName ?? ''}
          onChange={(e) => setGitKey(e.target.value || null)}
          title="SSH key used for fetch/pull/push"
          className="min-w-0 max-w-[130px] flex-none truncate rounded border border-koma-border bg-koma-bg px-1 py-0.5 font-mono text-[11px] text-koma-fg focus:outline-none focus:ring-1 focus:ring-koma-accent"
        >
          <option value="">Default (system ssh)</option>
          {keys.map((k) => (
            <option key={k.name} value={k.name}>
              {k.name}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-none flex-col gap-1.5 border-b border-koma-border px-3 py-2">
        <textarea
          value={commitDraft}
          onChange={(e) => setCommitDraft(e.target.value)}
          placeholder={`Message (${branchLabel})`}
          rows={2}
          className="w-full resize-none rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-50 focus:outline-none focus:ring-1 focus:ring-koma-accent"
        />
        <button
          type="button"
          disabled={!canCommit}
          onClick={() => gitCommit(commitDraft)}
          className="flex items-center justify-center gap-1.5 rounded bg-koma-accent px-3 py-1.5 text-[12px] font-semibold text-koma-bg transition-opacity disabled:cursor-not-allowed disabled:opacity-35"
        >
          <Check size={13} className="flex-none" />
          Commit
        </button>
      </div>
      <div className="flex flex-none items-center gap-1.5 border-b border-koma-border px-3 py-1.5">
        <Search size={12} className="flex-none text-koma-dim opacity-60" />
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter changed files"
          className="min-w-0 flex-1 bg-transparent font-mono text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-50 focus:outline-none"
        />
      </div>
      {clean ? (
        <Empty>No changes</Empty>
      ) : (
        <>
          {git.conflicted.length > 0 && (
            <AccordionSection
              title={`Conflicts (Merge Changes) · ${conflicted.length}`}
              open={open.conflicts}
              onToggle={() => setOpen((s) => ({ ...s, conflicts: !s.conflicts }))}
            >
              {conflicted.length === 0 ? (
                <Empty>No matching conflicts</Empty>
              ) : (
                conflicted.map((e) => (
                  <FileRow key={`c:${e.path}`} entry={e} onClick={() => openGitDiffTab(e.path, false)} />
                ))
              )}
            </AccordionSection>
          )}
          <AccordionSection
            title={staged.length === 0 ? 'Staged Changes' : `Staged Changes · ${staged.length}`}
            open={open.staged}
            onToggle={() => setOpen((s) => ({ ...s, staged: !s.staged }))}
            action={
              staged.length > 0 && (
                <HeaderAction
                  title="Unstage All"
                  onClick={() => gitUnstage(staged.map((e) => e.path))}
                >
                  <Minus size={13} />
                </HeaderAction>
              )
            }
          >
            {staged.length === 0 ? (
              <Empty>No staged changes</Empty>
            ) : (
              staged.map((e) => (
                <FileRow
                  key={`s:${e.path}`}
                  entry={e}
                  onClick={() => openGitDiffTab(e.path, true)}
                  onUnstage={() => gitUnstage([e.path])}
                />
              ))
            )}
          </AccordionSection>
          <AccordionSection
            title={unstaged.length === 0 ? 'Changes' : `Changes · ${unstaged.length}`}
            open={open.unstaged}
            onToggle={() => setOpen((s) => ({ ...s, unstaged: !s.unstaged }))}
            action={
              unstaged.length > 0 && (
                <span className="flex items-center gap-0.5">
                  <HeaderAction title="Discard All Changes" onClick={() => setArmedDiscard(DISCARD_ALL)}>
                    <Undo2 size={13} />
                  </HeaderAction>
                  <HeaderAction title="Stage All" onClick={() => gitStage(unstaged.map((e) => e.path))}>
                    <Plus size={13} />
                  </HeaderAction>
                </span>
              )
            }
          >
            {unstaged.length === 0 ? (
              <Empty>No changes</Empty>
            ) : (
              <>
                {armedDiscard === DISCARD_ALL && (
                  <DiscardConfirmRow
                    label={`Discard all ${unstaged.length} change(s)?`}
                    onConfirm={() => {
                      gitDiscard(unstaged.map((e) => e.path))
                      setArmedDiscard(null)
                    }}
                    onCancel={() => setArmedDiscard(null)}
                  />
                )}
                {unstaged.map((e) =>
                  armedDiscard === e.path ? (
                    <DiscardConfirmRow
                      key={`u:${e.path}`}
                      label={`Discard changes to ${baseName(e.path)}?`}
                      onConfirm={() => {
                        gitDiscard([e.path])
                        setArmedDiscard(null)
                      }}
                      onCancel={() => setArmedDiscard(null)}
                    />
                  ) : (
                    <FileRow
                      key={`u:${e.path}`}
                      entry={e}
                      onClick={() => openGitDiffTab(e.path, false)}
                      onStage={() => gitStage([e.path])}
                      onDiscard={() => setArmedDiscard(e.path)}
                    />
                  ),
                )}
              </>
            )}
          </AccordionSection>
        </>
      )}
      <AccordionSection
        title="Commit Graph"
        open={open.graph}
        onToggle={() => setOpen((s) => ({ ...s, graph: !s.graph }))}
        action={
          <HeaderAction title="Explore full graph" onClick={openGraphTab}>
            <Maximize2 size={13} />
          </HeaderAction>
        }
      >
        <GitGraphMini />
      </AccordionSection>
    </div>
  )
}
