import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, Copy, GitBranchPlus, GitCommitHorizontal } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { GitRef } from '../store/koma'
import { remoteShortName } from '../lib/gitRefs'

// What was right-clicked: a commit ROW (identified by sha) or a REF CHIP on a
// row (identified by the branch name it decorates, plus its `refKind` — tags
// aren't switchable, so a tag chip never opens this menu, see GraphRow's
// `onRefContextMenu` gating — a REMOTE ref's `name` is the full
// `<remote>/<branch>` form, e.g. "origin/feature", which must be short-named
// via `remoteShortName` before checkout so it DWIMs into a tracking branch
// instead of detaching HEAD).
export type GraphMenuTarget =
  | { kind: 'commit'; sha: string }
  | { kind: 'ref'; name: string; refKind: GitRef['kind'] }

type Props = {
  x: number
  y: number
  target: GraphMenuTarget
  onClose: () => void
}

const MENU_WIDTH = 220

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

function MenuItem({ icon, onClick, children }: { icon: ReactNode; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </button>
  )
}

// The commit-graph's right-click context menu (G4 — safe branch ops only, no
// cherry-pick/merge/rebase/reset/revert). Rendered by GraphTab, positioned at
// the click point (clamped to the viewport), closed on outside-click/Esc —
// mirrors BranchSwitcher's popover idiom. Two modes:
// - `kind: 'commit'` — "Checkout commit" (detached; confirmed inline, since
//   detached HEAD is surprising), "Create branch here…" (inline name input),
//   "Copy SHA".
// - `kind: 'ref'` — "Checkout <branch>", "Copy branch name".
export function GraphContextMenu({ x, y, target, onClose }: Props) {
  const gitCheckout = useKoma((s) => s.gitCheckout)
  const gitCreateBranch = useKoma((s) => s.gitCreateBranch)
  const [confirmCheckout, setConfirmCheckout] = useState(false)
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')

  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedPos(x, y, menuRef, [confirmCheckout, creating])

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
      {creating ? (
        <div className="flex items-center gap-1 px-2 py-1.5">
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitCreate()
              if (e.key === 'Escape') {
                e.stopPropagation()
                setCreating(false)
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
      ) : target.kind === 'commit' && confirmCheckout ? (
        <div className="flex flex-col gap-1.5 px-2.5 py-1.5">
          <span className="text-[11px] text-koma-dim">Checkout detaches HEAD. Continue?</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              autoFocus
              onClick={() => {
                gitCheckout(target.sha)
                onClose()
              }}
              className="flex flex-1 items-center justify-center gap-1 rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/25"
            >
              Checkout
            </button>
            <button
              type="button"
              onClick={() => setConfirmCheckout(false)}
              className="flex-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : target.kind === 'commit' ? (
        <>
          <MenuItem icon={<GitCommitHorizontal size={13} />} onClick={() => setConfirmCheckout(true)}>
            Checkout commit
          </MenuItem>
          <MenuItem icon={<GitBranchPlus size={13} />} onClick={() => setCreating(true)}>
            Create branch here…
          </MenuItem>
          <MenuItem icon={<Copy size={13} />} onClick={() => copy(target.sha)}>
            Copy SHA
          </MenuItem>
        </>
      ) : (
        <>
          {target.refKind !== 'tag' && (
            <MenuItem
              icon={<GitCommitHorizontal size={13} />}
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
        </>
      )}
    </div>,
    document.body,
  )
}
