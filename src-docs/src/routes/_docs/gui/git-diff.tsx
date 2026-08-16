import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/git-diff')({
  component: GuiGitDiffPage,
})

function GuiGitDiffPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Git &amp; Diff</h1>
      <p className="mb-6 text-koma-fg">
        The Source Control panel provides a visual interface for git operations,
        and the Monaco diff viewer shows side-by-side file changes.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Source Control Panel</h3>
          <p>
            The panel lists changed files grouped by staged / unstaged /
            untracked. Each file shows its status letter (M, A, D, ?).
            Click a file to open it in the diff viewer.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Stage / Unstage / Discard</h3>
          <p>
            Stage individual files or all changes with the stage button.
            Unstage with the unstage button. Discard changes to revert a
            file to its last committed state (confirmation required).
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Commits</h3>
          <p>
            Type a commit message in the input field and click Commit.
            The panel shows the number of staged files and total lines
            changed. Amend the last commit with the amend toggle.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Branch Switching</h3>
          <p>
            The branch selector at the top of the panel lists local branches.
            Click one to switch. Create new branches from the branch menu.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Sync &amp; Stash</h3>
          <p>
            Pull and push buttons sync with the remote. Stash current changes
            with the stash button; pop or drop stashes from the stash list.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Conflict Banner</h3>
          <p>
            When a merge or pull produces conflicts, a banner lists the
            conflicting files. Click a file to open the merge editor with
            conflict markers highlighted.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Monaco Diff Viewer</h3>
          <p>
            Clicking a changed file opens a side-by-side diff in Monaco.
            Additions are highlighted in green, deletions in red. Inline
            comments and chunk navigation are supported.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Commit Graph</h3>
          <p>
            The commit history view shows a branch graph with commit hashes,
            authors, dates, and message summaries. Navigate up and down
            the log with keyboard or scroll.
          </p>
        </div>
      </div>
    </article>
  )
}
