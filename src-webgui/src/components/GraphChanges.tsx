import { useEffect, useState } from 'react'
import { Check, Minus, Plus, Undo2 } from 'lucide-react'
import { AccordionSection } from './AccordionSection'
import { Empty } from './panels/helpers'
import { useKoma } from '../store/koma'
import { DiscardConfirmRow, FileRow, HeaderAction, baseName } from './GitChangesShared'

// GK3: the graph tab's left sidebar "Changes" accordion — a single parent
// section (commit box + Conflicts/Staged/unstaged sub-accordions) that ports
// GitPanel's stage/unstage/discard/commit idiom into the graph tab so working-
// tree edits are reachable without leaving the graph. Reuses the exact same
// row/badge/confirm atoms as GitPanel (see GitChangesShared.tsx) — GitPanel
// itself is untouched this wave.
export function GraphChanges() {
  const [open, setOpen] = useState({ changes: true, conflicts: true, staged: true, unstaged: true })
  // The single armed discard confirm across BOTH the per-row buttons and the
  // section-wide "Discard All Changes" action — a sentinel string for the
  // latter (no real path collides with it). Mirrors GitPanel's local state.
  const [armedDiscard, setArmedDiscard] = useState<string | null>(null)
  const DISCARD_ALL = '\0discard-all'

  const git = useKoma((s) => s.git)
  const commitDraft = useKoma((s) => s.commitDraft)
  const setCommitDraft = useKoma((s) => s.setCommitDraft)
  const refreshGitStatus = useKoma((s) => s.refreshGitStatus)
  const openGitDiffTab = useKoma((s) => s.openGitDiffTab)
  const gitStage = useKoma((s) => s.gitStage)
  const gitUnstage = useKoma((s) => s.gitUnstage)
  const gitDiscard = useKoma((s) => s.gitDiscard)
  const gitCommit = useKoma((s) => s.gitCommit)

  // Ensure status is fresh when the graph tab lands on this sidebar — the
  // graph tab is auto-opened (unlike GitPanel, which refreshes on its own
  // mount/reactivation), so this is the belt-and-suspenders refresh for the
  // "opened the graph without ever having visited Source Control" path.
  // Idempotent: a no-op re-fetch if the sidebar already refreshed recently.
  useEffect(() => {
    refreshGitStatus()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const { staged, unstaged, conflicted } = git
  const clean = staged.length === 0 && unstaged.length === 0 && conflicted.length === 0
  const canCommit = commitDraft.trim().length > 0 && staged.length > 0
  const branchLabel = git.detached ? 'detached HEAD' : (git.branch ?? '(unknown)')

  return (
    <AccordionSection
      title="Changes"
      open={open.changes}
      onToggle={() => setOpen((s) => ({ ...s, changes: !s.changes }))}
    >
      <div className="flex flex-none flex-col gap-1.5 px-3 py-2">
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

      {clean ? (
        <Empty>No changes</Empty>
      ) : (
        <>
          {conflicted.length > 0 && (
            <AccordionSection
              title={`Conflicts · ${conflicted.length}`}
              open={open.conflicts}
              onToggle={() => setOpen((s) => ({ ...s, conflicts: !s.conflicts }))}
            >
              {conflicted.map((e) => (
                <FileRow key={`c:${e.path}`} entry={e} onClick={() => openGitDiffTab(e.path, false)} />
              ))}
            </AccordionSection>
          )}
          <AccordionSection
            title={staged.length === 0 ? 'Staged Changes' : `Staged Changes · ${staged.length}`}
            open={open.staged}
            onToggle={() => setOpen((s) => ({ ...s, staged: !s.staged }))}
            action={
              staged.length > 0 && (
                <HeaderAction title="Unstage All" onClick={() => gitUnstage(staged.map((e) => e.path))}>
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
    </AccordionSection>
  )
}
