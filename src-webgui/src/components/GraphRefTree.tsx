import { useEffect, useState, type ReactNode } from 'react'
import { ArrowRightLeft, Cloud, GitBranch, Tag } from 'lucide-react'
import { AccordionSection } from './AccordionSection'
import { RowAction } from './GitChangesShared'
import { useKoma } from '../store/koma'
import type { BranchInfo } from '../store/koma'
import { remoteShortName } from '../lib/gitRefs'

// One LOCAL/REMOTE/TAG row. Clicking it looks up a loaded commit carrying a
// matching ref decoration (see `pick` in GraphRefTree) — a graceful no-op
// when that ref's commit isn't in the currently-loaded graph page. A hover
// "checkout" affordance (local/remote only — checking out a tag detaches
// HEAD, so tags never get one) reuses GitChangesShared's `RowAction` atom,
// the same subtle row-hover button the sidebar Source Control panel uses.
function RefRow({
  b,
  icon,
  onClick,
  onCheckout,
}: {
  b: BranchInfo
  icon: ReactNode
  onClick: () => void
  onCheckout?: () => void
}) {
  return (
    <div
      onClick={onClick}
      title={b.name}
      className="group flex min-h-[26px] cursor-pointer items-center gap-1.5 px-3 py-0.5 hover:bg-koma-hover"
    >
      {icon}
      <span
        className={`min-w-0 flex-1 truncate font-mono text-[12px] ${
          b.isCurrent ? 'font-semibold text-koma-accent' : 'text-koma-fg'
        }`}
      >
        {b.name}
      </span>
      {/* Current-branch marker — a small filled dot, mirroring the accent
          tone BranchSwitcher/GraphRow already use for "this is HEAD". */}
      {b.isCurrent && <span className="h-1.5 w-1.5 flex-none rounded-full bg-koma-accent" />}
      {onCheckout && (
        <RowAction title={`Checkout ${b.name}`} onClick={onCheckout}>
          <ArrowRightLeft size={12} />
        </RowAction>
      )}
    </div>
  )
}

type Props = {
  // Smooth-scroll the graph to a sha already resolved from a clicked ref —
  // lifted to GraphTab (owns `scrollerRef`/`rows`); see `scrollToSha` there.
  scrollToSha: (sha: string) => void
}

// The graph tab's left-sidebar LOCAL/REMOTE/TAGS ref-tree (GK4b) — now the
// sole occupant of that sidebar column (see GraphTab's sidebar column; the
// GK3 Changes accordion was removed so working-tree changes live only in
// the sidebar Source Control panel). Reuses the global `branches` slice (GitBranchList — G4 +
// GK4a's tag addition) as the AUTHORITATIVE full ref list, rather than
// re-deriving refs from the loaded commit page: every branch/tag exists
// whether or not its tip commit happens to be inside the currently-loaded
// graph window. Clicking a row then looks up a loaded commit that carries a
// matching ref decoration to jump to; `refreshBranches()` fires on mount so
// the tree populates even if no other surface (BranchSwitcher/GitPanel) has
// fetched it yet — idempotent, a no-op re-fetch if one already has.
export function GraphRefTree({ scrollToSha }: Props) {
  const [open, setOpen] = useState({ local: true, remote: false, tag: false })
  const branches = useKoma((s) => s.branches)
  const refreshBranches = useKoma((s) => s.refreshBranches)
  const commits = useKoma((s) => s.graph.commits)
  const selectCommit = useKoma((s) => s.selectCommit)
  const gitCheckout = useKoma((s) => s.gitCheckout)

  useEffect(() => {
    refreshBranches()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const locals = branches.filter((b) => b.kind === 'local')
  const remotes = branches.filter((b) => b.kind === 'remote')
  const tags = branches.filter((b) => b.kind === 'tag')

  // A branch's own commit is decorated `kind: 'local'` in the loaded graph —
  // EXCEPT the CURRENT branch, whose decoration comes back as `kind: 'head'`
  // (`HEAD -> refs/heads/X`, see host `git_graph.rs::parse_refs`) rather than
  // `local`, so a local row must also accept a `head` decoration by the same
  // name. Remote/tag rows match their own kind as-is (never rewritten by the
  // host to `head`).
  const pick = (b: BranchInfo) => {
    const commit = commits.find((c) =>
      c.refs.some((r) => r.name === b.name && (r.kind === b.kind || (b.kind === 'local' && r.kind === 'head'))),
    )
    if (!commit) return
    selectCommit(commit.sha)
    scrollToSha(commit.sha)
  }

  const checkoutOccupied = (b: BranchInfo) => b.worktreePath
    ?? (b.kind === 'remote' ? locals.find((local) => local.name === remoteShortName(b.name))?.worktreePath : undefined)

  const checkout = (b: BranchInfo) => {
    gitCheckout(b.kind === 'remote' ? remoteShortName(b.name) : b.name)
  }

  return (
    <>
      {locals.length > 0 && (
        <AccordionSection
          title={`Local · ${locals.length}`}
          open={open.local}
          onToggle={() => setOpen((s) => ({ ...s, local: !s.local }))}
        >
          {locals.map((b) => (
            <RefRow
              key={`l:${b.name}`}
              b={b}
              icon={<GitBranch size={13} className="flex-none text-koma-fg opacity-45" />}
              onClick={() => pick(b)}
              // Already checked out — nothing to switch to.
              onCheckout={b.isCurrent || b.worktreePath ? undefined : () => checkout(b)}
            />
          ))}
        </AccordionSection>
      )}
      {remotes.length > 0 && (
        <AccordionSection
          title={`Remote · ${remotes.length}`}
          open={open.remote}
          onToggle={() => setOpen((s) => ({ ...s, remote: !s.remote }))}
        >
          {remotes.map((b) => (
            <RefRow
              key={`r:${b.name}`}
              b={b}
              icon={<Cloud size={13} className="flex-none text-koma-fg opacity-45" />}
              onClick={() => pick(b)}
              onCheckout={checkoutOccupied(b) ? undefined : () => checkout(b)}
            />
          ))}
        </AccordionSection>
      )}
      {tags.length > 0 && (
        <AccordionSection
          title={`Tags · ${tags.length}`}
          open={open.tag}
          onToggle={() => setOpen((s) => ({ ...s, tag: !s.tag }))}
        >
          {tags.map((b) => (
            <RefRow
              key={`t:${b.name}`}
              b={b}
              icon={<Tag size={13} className="flex-none text-koma-fg opacity-45" />}
              onClick={() => pick(b)}
              // A tag isn't switchable without detaching HEAD — no checkout
              // affordance on a tag row.
            />
          ))}
        </AccordionSection>
      )}
    </>
  )
}
