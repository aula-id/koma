import { useEffect, useMemo, useRef, useState, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, GitBranch, Plus, Search } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { BranchInfo } from '../store/koma'
import { remoteShortName } from '../lib/gitRefs'

// Track the trigger button's viewport rect while the popover is open, so it
// can render in a body portal (fixed positioning) that no `overflow` ancestor
// can clip. Mirrors NewSessionMenu's `useAnchorRect`.
function useAnchorRect<T extends HTMLElement>(open: boolean, ref: RefObject<T | null>) {
  const [rect, setRect] = useState<DOMRect | null>(null)
  useEffect(() => {
    if (!open) {
      setRect(null)
      return
    }
    const update = () => {
      if (ref.current) setRect(ref.current.getBoundingClientRect())
    }
    update()
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [open, ref])
  return rect
}

const MENU_MAX_HEIGHT = 360

// Clamp the popover's vertical position to the viewport, mirroring
// GraphContextMenu's `useClampedPos` two-axis clamp — the raw anchor (opens
// upward from the trigger for the 'footer' variant, downward for 'icon')
// re-measures against the menu's real height after mount, since an
// icon-triggered popover near the window bottom would otherwise render
// (partially) offscreen.
function useClampedVPos(
  rect: DOMRect | null,
  variant: 'footer' | 'icon',
  ref: RefObject<HTMLDivElement | null>,
  deps: unknown[],
) {
  const rawTop = (h: number) => (rect ? (variant === 'footer' ? rect.top - 4 - h : rect.bottom + 4) : 0)
  const [top, setTop] = useState(() => rawTop(MENU_MAX_HEIGHT))
  useEffect(() => {
    if (!rect) return
    const h = ref.current?.offsetHeight ?? MENU_MAX_HEIGHT
    setTop(Math.max(8, Math.min(rawTop(h), window.innerHeight - h - 8)))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rect, variant, ref, ...deps])
  return top
}

function BranchRow({ b, onPick }: { b: BranchInfo; onPick: (b: BranchInfo) => void }) {
  return (
    <button
      type="button"
      disabled={b.isCurrent}
      onClick={() => onPick(b)}
      title={b.name}
      className={`flex w-full items-center gap-1.5 px-2.5 py-1 text-left font-mono text-[12px] ${
        b.isCurrent
          ? 'cursor-default text-koma-accent opacity-90'
          : 'text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      {b.isCurrent ? <Check size={12} className="flex-none" /> : <span className="w-3 flex-none" />}
      <span className="min-w-0 flex-1 truncate">{b.name}</span>
    </button>
  )
}

type BranchSwitcherProps = {
  // 'footer' renders the UsageFooter's branch-name + icon trigger (the caller
  // gates this on a resolved, non-detached branch); 'icon' renders a bare
  // icon-only trigger for the GitPanel header switch button.
  variant: 'footer' | 'icon'
}

// Branch-switcher popover (G4 — safe branch ops only): a filterable
// local-then-remote branch list (current branch marked + disabled) plus an
// inline "+ Create new branch" row. Self-contained trigger + portaled menu,
// mirroring NewSessionMenu's anchor + outside-click/Esc idiom. Shared by
// UsageFooter's branch indicator and GitPanel's header switch icon.
export function BranchSwitcher({ variant }: BranchSwitcherProps) {
  const gitBranch = useKoma((s) => s.git.branch)
  const branches = useKoma((s) => s.branches)
  const branchesLoading = useKoma((s) => s.branchesLoading)
  const refreshBranches = useKoma((s) => s.refreshBranches)
  const gitCheckout = useKoma((s) => s.gitCheckout)
  const gitCreateBranch = useKoma((s) => s.gitCreateBranch)

  const [open, setOpen] = useState(false)
  const [filter, setFilter] = useState('')
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const rect = useAnchorRect(open, triggerRef)

  // Fire a fresh fetch every time the popover opens (mirrors GitPanel's
  // mount-time refreshGitStatus idiom) so the list is never stale.
  useEffect(() => {
    if (open) refreshBranches()
  }, [open, refreshBranches])

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (triggerRef.current?.contains(t) || menuRef.current?.contains(t)) return
      setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  // Reset the transient filter/create-draft every time the popover closes, so
  // reopening never shows stale half-typed state.
  useEffect(() => {
    if (!open) {
      setFilter('')
      setCreating(false)
      setNewName('')
    }
  }, [open])

  // Tags (GK4a — listed alongside branches for the graph's ref-tree) aren't
  // switchable from here: `git checkout <tag>` detaches HEAD, which this
  // popover's SAFE-only branch-pick idiom doesn't offer — the ref-tree TAGS
  // group is the place for those. Filter them out before the local/remote
  // split below.
  const switchable = useMemo(() => branches.filter((b) => b.kind !== 'tag'), [branches])
  const q = filter.trim().toLowerCase()
  const filtered = q ? switchable.filter((b) => b.name.toLowerCase().includes(q)) : switchable
  const locals = useMemo(() => filtered.filter((b) => b.kind === 'local'), [filtered])
  const remotes = useMemo(() => filtered.filter((b) => b.kind === 'remote'), [filtered])

  const pick = (b: BranchInfo) => {
    if (b.isCurrent) return
    gitCheckout(b.kind === 'remote' ? remoteShortName(b.name) : b.name)
    setOpen(false)
  }

  const submitCreate = () => {
    if (!newName.trim()) return
    gitCreateBranch(newName.trim(), null, true)
    setOpen(false)
  }

  const menuWidth = 240
  const vTop = useClampedVPos(rect, variant, menuRef, [branchesLoading, filter, creating, locals.length, remotes.length])

  return (
    <span className="relative flex flex-none items-center">
      {variant === 'footer' ? (
        <button
          ref={triggerRef}
          type="button"
          onClick={() => setOpen((o) => !o)}
          title={`git branch: ${gitBranch ?? ''} (click to switch)`}
          className="flex flex-none items-center gap-1 rounded px-0.5 opacity-70 transition hover:bg-koma-hover hover:opacity-100"
        >
          <GitBranch size={11} className="flex-none" />
          <span className="max-w-[140px] truncate">{gitBranch}</span>
        </button>
      ) : (
        <button
          ref={triggerRef}
          type="button"
          onClick={() => setOpen((o) => !o)}
          title="Switch branch"
          aria-label="Switch branch"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <GitBranch size={13} />
        </button>
      )}
      {open &&
        rect &&
        createPortal(
          <div
            ref={menuRef}
            style={{
              position: 'fixed',
              // The footer sits at the window bottom, so the popover opens
              // UPWARD from the trigger; every other surface (GitPanel header)
              // opens downward, mirroring NewSessionMenu — `vTop` (from
              // `useClampedVPos`) already picks the right anchor direction AND
              // clamps to the viewport, so an icon-triggered popover near the
              // window bottom never renders offscreen.
              top: vTop,
              left: Math.max(8, Math.min(rect.left, window.innerWidth - menuWidth - 8)),
              width: menuWidth,
              maxHeight: MENU_MAX_HEIGHT,
              zIndex: 80,
            }}
            className="flex flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-sm"
          >
            <div className="flex flex-none items-center gap-1.5 border-b border-koma-border px-2 py-1.5">
              <Search size={11} className="flex-none text-koma-dim opacity-60" />
              <input
                autoFocus
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Filter branches"
                className="min-w-0 flex-1 bg-transparent font-mono text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-50 focus:outline-none"
              />
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto py-1">
              {branchesLoading && branches.length === 0 ? (
                <div className="px-2.5 py-2 text-[11px] text-koma-dim opacity-70">Loading…</div>
              ) : (
                <>
                  {locals.length > 0 && (
                    <div className="px-2.5 pb-0.5 pt-1 text-[10px] font-semibold uppercase tracking-wide text-koma-dim opacity-60">
                      Local
                    </div>
                  )}
                  {locals.map((b) => (
                    <BranchRow key={`l:${b.name}`} b={b} onPick={pick} />
                  ))}
                  {remotes.length > 0 && (
                    <div className="px-2.5 pb-0.5 pt-1.5 text-[10px] font-semibold uppercase tracking-wide text-koma-dim opacity-60">
                      Remote
                    </div>
                  )}
                  {remotes.map((b) => (
                    <BranchRow key={`r:${b.name}`} b={b} onPick={pick} />
                  ))}
                  {!branchesLoading && locals.length === 0 && remotes.length === 0 && (
                    <div className="px-2.5 py-2 text-[11px] text-koma-dim opacity-70">No branches</div>
                  )}
                </>
              )}
            </div>
            <div className="flex-none border-t border-koma-border p-1.5">
              {creating ? (
                <div className="flex items-center gap-1">
                  <input
                    autoFocus
                    value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') submitCreate()
                      if (e.key === 'Escape') {
                        e.stopPropagation()
                        setCreating(false)
                        setNewName('')
                      }
                    }}
                    placeholder="New branch name"
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
              ) : (
                <button
                  type="button"
                  onClick={() => setCreating(true)}
                  className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
                >
                  <Plus size={13} className="flex-none" />
                  Create new branch
                </button>
              )}
            </div>
          </div>,
          document.body,
        )}
    </span>
  )
}
