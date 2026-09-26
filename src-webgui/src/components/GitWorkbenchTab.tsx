import { GitBranch } from 'lucide-react'
import type { GitToolTab } from '../lib/gitWorkbench'
import { useGitWork } from './gitWorkbenchShared'
import { GitBlameView, GitConflictView, GitDiffView } from './GitFileTools'
import { GitBranchesView, GitRebaseView, GitReflogView, GitRemotesView, GitStashesView, GitTagsView } from './GitRepositoryTools'
import { BrailleSpinner } from './BrailleSpinner'

export default function GitWorkbenchTab({ tab }: { tab: GitToolTab }) {
  const work = useGitWork(tab.root)
  return <div className="flex h-full min-h-0 min-w-0 flex-col bg-koma-bg">
    <div className="flex h-8 min-w-0 flex-none items-center gap-2 border-b border-koma-border bg-koma-panel px-3 text-[12px]">
      <GitBranch size={13} className="flex-none text-koma-dim" /><span className="min-w-0 flex-1 truncate font-mono text-koma-fg" title={tab.root}>{tab.path ?? tab.title}</span>
      <span className="max-w-[40%] truncate text-[11px] text-koma-dim" title={tab.root}>{tab.root.split(/[\\/]/).pop()}</span>{work.busy && <BrailleSpinner size={13} />}
    </div>
    {!work.active && <div className="border-b border-koma-warn/40 px-3 py-2 text-[11px] text-koma-warn">Select this repository in Source Control before using its Git actions.</div>}
    {work.error && <div role="alert" className="max-h-36 flex-none overflow-auto whitespace-pre-wrap break-words border-b border-koma-error/40 px-3 py-2 text-[11px] text-koma-error">{work.error}</div>}
    {tab.view === 'diff' ? <GitDiffView tab={tab} work={work} /> : tab.view === 'resolve' ? <GitConflictView tab={tab} work={work} /> : tab.view === 'blame' ? <GitBlameView tab={tab} work={work} /> : tab.view === 'branches' ? <GitBranchesView tab={tab} work={work} /> : tab.view === 'tags' ? <GitTagsView tab={tab} work={work} /> : tab.view === 'remotes' ? <GitRemotesView work={work} /> : tab.view === 'stashes' ? <GitStashesView tab={tab} work={work} /> : tab.view === 'reflog' ? <GitReflogView work={work} /> : <GitRebaseView tab={tab} work={work} />}
  </div>
}
