import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import {
  AlertTriangle,
  Check,
  Cherry,
  Copy,
  GitBranchPlus,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequestArrow,
  RotateCcw,
  Undo2,
} from 'lucide-react'
import { useKoma } from '../store/koma'
import type { GitRef } from '../store/koma'
import { remoteShortName } from '../lib/gitRefs'

// What was right-clicked: a commit ROW (identified by sha) or a REF CHIP on a
// row (identified by the branch name it decorates, plus its `refKind` — tags
// aren't switchable, so a tag chip never opens this menu, see GraphRow's
// `onRefContextMenu` gating — a REMOTE ref's `name` is the full
// `<remote>/<branch>` form, e.g. "origin/feature", which must be short-named
// via `remoteShortName` before CHECKOUT so it DWIMs into a tracking branch
// instead of detaching HEAD. Merge/rebase must NOT do this strip: a remote-
// tracking ref and its local counterpart can point at different commits
// after a fetch, so they use the ref name as-is to target the exact ref the
// user clicked).
export type GraphMenuTarget =
  | { kind: 'commit'; sha: string }
  | { kind: 'ref'; name: string; refKind: GitRef['kind'] }

type Props = {
  x: number
  y: number
  target: GraphMenuTarget
  onClose: () => void
}

const MENU_WIDTH = 240

// Every menu "mode" — the plain item list ('menu'), the create-branch inline
// input ('creating'), or one of the inline confirms for a destructive/
// conflict-capable op. Only one is ever shown at a time (this is a single
// popup, never nested).
type Mode =
  | 'menu'
  | 'creating'
  | 'confirmCheckout'
  | 'confirmCherryPick'
  | 'confirmRevert'
  | 'resetPick'
  | 'confirmReset'
  | 'confirmMerge'
  | 'confirmRebase'

// Clamp the raw viewport (x, y) click point so the menu never renders
// off-screen — re-measured after mount since the menu's real height depends
// on which target/mode it's showing (commit vs ref, plain vs confirm/create).
function useClampedPos(x: number, y: number, ref: RefObject<HTMLDivElement | null>, deps: unknown[]) {
  const [pos, setPos] = useState({ left: x, top: y })
  useEffect(() => {
    const el = ref.current
    const w = el?.offsetWidth ?? MENU_WIDTH
    const h = el?.offsetHeight ?? 140
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - w - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - h - 4)),
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [x, y, ref, ...deps])
  return pos
}

function MenuItem({
  icon,
  onClick,
  disabled,
  danger,
  children,
}: {
  icon: ReactNode
  onClick: () => void
  disabled?: boolean
  danger?: boolean
  children: ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={disabled ? 'Unavailable in detached HEAD' : undefined}
      className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] transition-colors ${
        disabled
          ? 'cursor-not-allowed text-koma-dim opacity-40'
          : danger
            ? 'text-koma-error opacity-90 hover:bg-koma-error/10 hover:opacity-100'
            : 'text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </button>
  )
}

function Separator() {
  return <div className="my-1 border-t border-koma-border" />
}

// A generic inline confirm row (mirrors the pre-existing detach-checkout
// confirm idiom) — used for every conflict-capable/destructive op except
// `reset --hard` (which gets the stronger `HardResetConfirm` below).
function InlineConfirm({
  label,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  label: string
  confirmLabel: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div className="flex flex-col gap-1.5 px-2.5 py-1.5">
      <span className="text-[11px] text-koma-dim">{label}</span>
      <div className="flex items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          className="flex flex-1 items-center justify-center gap-1 rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/25"
        >
          {confirmLabel}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="flex-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}

// The EXTRA-strong confirm for `git reset --hard` — visually distinct from
// every other confirm here (bordered card, warning icon, explicit "discards
// ALL uncommitted changes" wording, Cancel auto-focused rather than the
// destructive action) since this is the one op in this menu that can
// silently destroy uncommitted work with no conflict/recovery path.
function HardResetConfirm({
  branch,
  onConfirm,
  onCancel,
}: {
  branch: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div className="flex flex-col gap-1.5 border border-koma-error/40 bg-koma-error/10 px-2.5 py-2">
      <div className="flex items-center gap-1.5 text-koma-error">
        <AlertTriangle size={13} className="flex-none" />
        <span className="text-[11px] font-semibold uppercase tracking-wide">Hard reset</span>
      </div>
      <span className="text-[11px] font-medium text-koma-error opacity-90">
        This DISCARDS ALL uncommitted changes on {branch} — working tree and index alike. This cannot be undone.
      </span>
      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={onConfirm}
          className="flex flex-1 items-center justify-center gap-1 rounded bg-koma-error px-2 py-1 text-[11px] font-bold text-koma-bg hover:opacity-90"
        >
          Discard &amp; reset
        </button>
        <button
          type="button"
          autoFocus
          onClick={onCancel}
          className="flex-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}

// The commit-graph's right-click context menu. Rendered by GraphTab,
// positioned at the click point (clamped to the viewport), closed on
// outside-click/Esc — mirrors BranchSwitcher's popover idiom. Two targets:
// - `kind: 'commit'` — safe ops (Checkout commit / Create branch here / Copy
//   SHA) plus the destructive/interactive ops (G5c): Cherry-pick, Revert,
//   Reset <branch> to here (soft/mixed/hard), Merge into <branch>, Rebase
//   <branch> onto here.
// - `kind: 'ref'` — Checkout <branch> / Copy branch name, plus (for a
//   local/remote branch chip, never a tag) Merge <branch> into <current> and
//   Rebase <current> onto <branch>.
// Every destructive/conflict-capable action is gated behind an inline
// confirm (never `window.confirm`) — reset --hard gets a visually distinct,
// extra-strong one. The reset/merge/rebase-onto-current items disable
// themselves (with a tooltip) while HEAD is detached, since there's no
// "current branch" for them to act on.
export function GraphContextMenu({ x, y, target, onClose }: Props) {
  const gitCheckout = useKoma((s) => s.gitCheckout)
  const gitCreateBranch = useKoma((s) => s.gitCreateBranch)
  const gitCherryPick = useKoma((s) => s.gitCherryPick)
  const gitRevert = useKoma((s) => s.gitRevert)
  const gitReset = useKoma((s) => s.gitReset)
  const gitMerge = useKoma((s) => s.gitMerge)
  const gitRebase = useKoma((s) => s.gitRebase)
  const branch = useKoma((s) => s.git.branch)
  const detached = useKoma((s) => s.git.detached)
  const branches = useKoma((s) => s.branches)
  const currentBranch = detached ? null : branch
  const targetOccupied = target.kind === 'ref' && target.refKind !== 'tag'
    && branches.some((b) => b.kind === 'local'
      && b.name === (target.refKind === 'remote' ? remoteShortName(target.name) : target.name)
      && !!b.worktreePath && !b.isCurrent)

  const [mode, setMode] = useState<Mode>('menu')
  const [newName, setNewName] = useState('')
  const [resetMode, setResetMode] = useState<'soft' | 'mixed' | 'hard'>('mixed')

  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedPos(x, y, menuRef, [mode, resetMode])

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    // Capture-phase mousedown so a stray click elsewhere (including another
    // row's context-menu trigger) always closes this one first.
    window.addEventListener('mousedown', onDoc, true)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc, true)
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  const copy = (text: string) => {
    void navigator.clipboard?.writeText(text)
    onClose()
  }

  const startPoint = target.kind === 'commit' ? target.sha : target.name
  // The commit-kind confirms (cherry-pick/revert/reset) only ever fire from
  // the commit-kind item list below, so `sha` is guaranteed non-null whenever
  // those modes are actually reachable — kept as a plain nullable rather than
  // narrowing on `mode` (TS can't correlate the two independent states).
  const sha = target.kind === 'commit' ? target.sha : null
  // Merge/rebase source: a commit row's own sha, or a ref chip's name AS-IS.
  // Unlike checkout, do NOT short-name a remote chip here — `origin/main` is
  // a distinct remote-tracking ref from local `main` (they can point at
  // different commits after a fetch), so merging/rebasing must target the
  // ref the user actually clicked, not a DWIM'd local branch.
  const mergeRebaseSource = target.kind === 'commit' ? target.sha : target.name
  const mergeRebaseLabel =
    target.kind === 'commit' ? `commit ${target.sha.slice(0, 7)}` : mergeRebaseSource

  const submitCreate = () => {
    if (!newName.trim()) return
    gitCreateBranch(newName.trim(), startPoint, true)
    onClose()
  }

  return createPortal(
    <div
      ref={menuRef}
      style={{ position: 'fixed', left: pos.left, top: pos.top, width: MENU_WIDTH, zIndex: 90 }}
      className="flex flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
      onContextMenu={(e) => e.preventDefault()}
    >
      {mode === 'creating' ? (
        <div className="flex items-center gap-1 px-2 py-1.5">
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitCreate()
              if (e.key === 'Escape') {
                e.stopPropagation()
                setMode('menu')
              }
            }}
            placeholder="Branch name"
            className="min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 py-1 font-mono text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-50 focus:outline-none focus:ring-1 focus:ring-koma-accent"
          />
          <button
            type="button"
            disabled={!newName.trim()}
            onClick={submitCreate}
            aria-label="Create branch"
            className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-30"
          >
            <Check size={13} />
          </button>
        </div>
      ) : mode === 'confirmCheckout' ? (
        <InlineConfirm
          label="Checkout detaches HEAD. Continue?"
          confirmLabel="Checkout"
          onConfirm={() => {
            if (sha) gitCheckout(sha)
            onClose()
          }}
          onCancel={() => setMode('menu')}
        />
      ) : mode === 'confirmCherryPick' ? (
        <InlineConfirm
          label="Cherry-pick this commit onto the current branch? May conflict."
          confirmLabel="Cherry-pick"
          onConfirm={() => {
            if (sha) gitCherryPick(sha)
            onClose()
          }}
          onCancel={() => setMode('menu')}
        />
      ) : mode === 'confirmRevert' ? (
        <InlineConfirm
          label="Revert this commit (creates a new commit undoing it)? May conflict."
          confirmLabel="Revert"
          onConfirm={() => {
            if (sha) gitRevert(sha)
            onClose()
          }}
          onCancel={() => setMode('menu')}
        />
      ) : mode === 'resetPick' ? (
        <div className="flex flex-col gap-1.5 px-2.5 py-1.5">
          <span className="text-[11px] text-koma-dim">Reset {currentBranch ?? 'branch'} to here:</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => {
                setResetMode('soft')
                setMode('confirmReset')
              }}
              className="flex-1 rounded px-2 py-1 text-[11px] font-semibold text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
            >
              Soft
            </button>
            <button
              type="button"
              onClick={() => {
                setResetMode('mixed')
                setMode('confirmReset')
              }}
              className="flex-1 rounded px-2 py-1 text-[11px] font-semibold text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
            >
              Mixed
            </button>
            <button
              type="button"
              onClick={() => {
                setResetMode('hard')
                setMode('confirmReset')
              }}
              className="flex-1 rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/25"
            >
              Hard
            </button>
          </div>
          <button
            type="button"
            onClick={() => setMode('menu')}
            className="rounded px-2 py-0.5 text-[11px] text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100"
          >
            Cancel
          </button>
        </div>
      ) : mode === 'confirmReset' ? (
        resetMode === 'hard' ? (
          <HardResetConfirm
            branch={currentBranch ?? 'the current branch'}
            onConfirm={() => {
              if (sha) gitReset(sha, 'hard')
              onClose()
            }}
            onCancel={() => setMode('resetPick')}
          />
        ) : (
          <InlineConfirm
            label={`Reset ${currentBranch ?? 'the current branch'} to here (${resetMode})?`}
            confirmLabel="Reset"
            onConfirm={() => {
              if (sha) gitReset(sha, resetMode)
              onClose()
            }}
            onCancel={() => setMode('resetPick')}
          />
        )
      ) : mode === 'confirmMerge' ? (
        <InlineConfirm
          label={`Merge ${mergeRebaseLabel} into ${currentBranch ?? 'the current branch'}? May conflict.`}
          confirmLabel="Merge"
          onConfirm={() => {
            gitMerge(mergeRebaseSource)
            onClose()
          }}
          onCancel={() => setMode('menu')}
        />
      ) : mode === 'confirmRebase' ? (
        <InlineConfirm
          label={`Rebase ${currentBranch ?? 'the current branch'} onto ${mergeRebaseLabel}? May conflict.`}
          confirmLabel="Rebase"
          onConfirm={() => {
            gitRebase(mergeRebaseSource)
            onClose()
          }}
          onCancel={() => setMode('menu')}
        />
      ) : target.kind === 'commit' ? (
        <>
          <MenuItem icon={<GitCommitHorizontal size={13} />} onClick={() => setMode('confirmCheckout')}>
            Checkout commit
          </MenuItem>
          <MenuItem icon={<GitBranchPlus size={13} />} onClick={() => setMode('creating')}>
            Create branch here…
          </MenuItem>
          <MenuItem icon={<Copy size={13} />} onClick={() => copy(target.sha)}>
            Copy SHA
          </MenuItem>
          <Separator />
          <MenuItem icon={<Cherry size={13} />} onClick={() => setMode('confirmCherryPick')}>
            Cherry-pick commit
          </MenuItem>
          <MenuItem icon={<Undo2 size={13} />} onClick={() => setMode('confirmRevert')}>
            Revert commit
          </MenuItem>
          <MenuItem icon={<RotateCcw size={13} />} disabled={!currentBranch} onClick={() => setMode('resetPick')}>
            {currentBranch ? `Reset ${currentBranch} to here` : 'Reset (detached HEAD)'}
          </MenuItem>
          <MenuItem icon={<GitMerge size={13} />} disabled={!currentBranch} onClick={() => setMode('confirmMerge')}>
            {currentBranch ? `Merge into ${currentBranch}` : 'Merge (detached HEAD)'}
          </MenuItem>
          <MenuItem
            icon={<GitPullRequestArrow size={13} />}
            disabled={!currentBranch}
            onClick={() => setMode('confirmRebase')}
          >
            {currentBranch ? `Rebase ${currentBranch} onto here` : 'Rebase (detached HEAD)'}
          </MenuItem>
        </>
      ) : (
        <>
          {target.refKind !== 'tag' && (
            <MenuItem
              icon={<GitCommitHorizontal size={13} />}
              disabled={targetOccupied}
              onClick={() => {
                // A remote chip's `name` is the full `<remote>/<branch>` form —
                // strip it via the shared `remoteShortName` helper (same one
                // BranchSwitcher uses) so the checkout DWIMs a local tracking
                // branch instead of detaching HEAD on the remote-qualified ref.
                gitCheckout(target.refKind === 'remote' ? remoteShortName(target.name) : target.name)
                onClose()
              }}
            >
              Checkout {target.refKind === 'remote' ? remoteShortName(target.name) : target.name}
            </MenuItem>
          )}
          <MenuItem icon={<Copy size={13} />} onClick={() => copy(target.name)}>
            Copy branch name
          </MenuItem>
          {target.refKind !== 'tag' && (
            <>
              <Separator />
              <MenuItem
                icon={<GitMerge size={13} />}
                disabled={!currentBranch}
                onClick={() => setMode('confirmMerge')}
              >
                {currentBranch ? `Merge ${mergeRebaseLabel} into ${currentBranch}` : 'Merge (detached HEAD)'}
              </MenuItem>
              <MenuItem
                icon={<GitPullRequestArrow size={13} />}
                disabled={!currentBranch}
                onClick={() => setMode('confirmRebase')}
              >
                {currentBranch ? `Rebase ${currentBranch} onto ${mergeRebaseLabel}` : 'Rebase (detached HEAD)'}
              </MenuItem>
            </>
          )}
        </>
      )}
    </div>,
    document.body,
  )
}
