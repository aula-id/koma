import { useEffect, useState } from 'react'
import { GitBranch, ArrowUp, ArrowDown, FileText, Search } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
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

// One GIT-panel file row: basename (+ dim parent dir), a rename shows
// `origPath -> path` instead, and a trailing status-char badge. Click opens
// the staged/unstaged Monaco diff for this path (read-only — no stage/
// unstage controls yet, that's Wave 3).
function FileRow({ entry, onClick }: { entry: GitFileEntry; onClick: () => void }) {
  const tone = STATUS_TONE[entry.status] ?? 'text-koma-dim'
  const dir = dirName(entry.path)
  return (
    <div
      title={entry.origPath ? `${entry.origPath} -> ${entry.path}` : entry.path}
      onClick={onClick}
      className="group flex min-h-[30px] cursor-pointer items-center gap-2.5 px-3 py-1 hover:bg-koma-hover"
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
      <span className={`flex-none font-mono text-[11px] font-semibold ${tone}`}>{entry.status}</span>
    </div>
  )
}

// VSCode-style Source Control panel — read-only for now (Wave 2): current
// branch + ahead/behind, a filter box, and Staged Changes / Changes
// accordions mirroring `git status --porcelain`'s staged/unstaged split.
// Stage/unstage/commit/discard controls are Wave 3.
export function GitPanel() {
  const [open, setOpen] = useState({ staged: true, unstaged: true })
  const [filter, setFilter] = useState('')
  const git = useKoma((s) => s.git)
  const refreshGitStatus = useKoma((s) => s.refreshGitStatus)
  const openGitDiffTab = useKoma((s) => s.openGitDiffTab)

  // Fetch fresh status on mount (panel opened/activated). GitPanel unmounts
  // when the sidebar switches to another view, so re-selecting "Source
  // Control" re-runs this — no separate "became active" plumbing needed.
  useEffect(() => {
    refreshGitStatus()
  }, [refreshGitStatus])

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
  const clean = git.staged.length === 0 && git.unstaged.length === 0
  const branchLabel = git.detached ? 'detached HEAD' : (git.branch ?? '(unknown)')

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
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
          <AccordionSection
            title={staged.length === 0 ? 'Staged Changes' : `Staged Changes · ${staged.length}`}
            open={open.staged}
            onToggle={() => setOpen((s) => ({ ...s, staged: !s.staged }))}
          >
            {staged.length === 0 ? (
              <Empty>No staged changes</Empty>
            ) : (
              staged.map((e) => (
                <FileRow key={`s:${e.path}`} entry={e} onClick={() => openGitDiffTab(e.path, true)} />
              ))
            )}
          </AccordionSection>
          <AccordionSection
            title={unstaged.length === 0 ? 'Changes' : `Changes · ${unstaged.length}`}
            open={open.unstaged}
            onToggle={() => setOpen((s) => ({ ...s, unstaged: !s.unstaged }))}
          >
            {unstaged.length === 0 ? (
              <Empty>No changes</Empty>
            ) : (
              unstaged.map((e) => (
                <FileRow key={`u:${e.path}`} entry={e} onClick={() => openGitDiffTab(e.path, false)} />
              ))
            )}
          </AccordionSection>
        </>
      )}
    </div>
  )
}
