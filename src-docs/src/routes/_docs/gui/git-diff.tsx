import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/git-diff')({
  component: GuiGitDiffPage,
})

const features = [
  {
    title: 'Stage, unstage, and compare',
    text: 'Click a changed file in Source Control to compare it. Select individual added/deleted lines or stage a complete hunk. In Staged Changes, the hunk view describes Index → HEAD so you can unstage the selected changes. Side by side shows the usual before/after diff. Neither operation edits the working file. Use the existing file buttons for renames, permission changes, binary files, and files with custom filters.',
  },
  {
    title: 'Commits and amend',
    text: 'Write a message and commit the staged files. Turn on Amend last commit to load the previous message and replace HEAD after confirmation. Amend includes staged changes only; you can also change only the message. It rewrites the commit identity, including for a commit that was already pushed.',
  },
  {
    title: 'Three-panel conflict editor',
    text: 'Open a file under Conflicts to see current and incoming above an editable result. Move between conflict blocks and accept current, incoming, or both, then refine the result. Save result writes the file; Mark resolved also stages it. Binary and deletion conflicts offer whole-file choices. During rebase, labels identify the rebased destination and the commit being replayed. Continue or Abort from the conflict banner.',
  },
  {
    title: 'Interactive rebase',
    text: 'Right-click a graph commit and choose Interactive rebase after here. The selected base is excluded. Reorder the subsequent commits through HEAD, or choose pick, reword, squash, and drop. Enter reword/squash messages before starting. This requires a clean working tree and a linear range of at most 500 commits, without merge commits. Koma creates a recovery reference before rewriting; Git keeps the sequence while conflicts are resolved.',
  },
  {
    title: 'Branches and tags',
    text: 'Use the Source Control management buttons or a graph reference’s Manage action to rename/delete local branches and explicitly delete remote branches. Current branches and branches occupied by a worktree cannot be deleted. Right-click a commit to create a lightweight or annotated tag there. Push tags or delete them on a chosen remote through separate confirmations.',
  },
  {
    title: 'Stash manager',
    text: 'Open Manage stashes from Source Control or the graph toolbar. Create a named stash, optionally including untracked files, and inspect each stash’s patch. Apply keeps the stash; Pop removes it after a successful apply; Drop discards it after confirmation. A pop that produces conflicts keeps the stash.',
  },
  {
    title: 'Reflog and recovery',
    text: 'The Reflog button opens local reference history with pagination. Select an entry to open its commit or create a recovery branch. Right-click for the existing graph actions, including reset with confirmation. Reflog records reference movements; it is not a general undo/redo history for every file operation.',
  },
  {
    title: 'Blame HEAD',
    text: 'Right-click a file in Coding, or choose Blame HEAD from a working diff, to open the committed file in a read-only editor with a narrow commit gutter. Click a commit to open it in the graph. Blame uses HEAD; Koma shows a notice when the working file differs.',
  },
  {
    title: 'Remotes',
    text: 'Manage remotes from Source Control to add, rename, edit fetch/push URLs, or remove a remote after confirmation. Saving a remote does not fetch automatically. Fetch, pull, push, and the existing SSH key selector remain in the usual toolbar.',
  },
  {
    title: 'Image comparisons',
    text: 'Image changes open before/after previews with dimensions, byte sizes, and Fit or Actual size. This works for working changes, staged changes, and historical commits, including added/deleted images. Supported formats are PNG, JPEG, GIF, WebP, BMP, ICO, AVIF, and SVG. Image pairs are limited to 16 MiB; text previews are limited to 2 MiB per side.',
  },
]

function GuiGitDiffPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Git &amp; Diff
      </h1>
      <p className="mb-6 text-koma-fg">
        Source Control and the commit graph share Git editors and management
        tabs for local and SSH projects. Tabs remain bound to the repository
        where they were opened; select that repository before using their
        actions.
      </p>
      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        {features.map(({ title, text }) => (
          <div key={title}>
            <h3 className="mb-1 text-base font-semibold text-koma-fg">
              {title}
            </h3>
            <p>{text}</p>
          </div>
        ))}
      </div>
    </article>
  )
}
