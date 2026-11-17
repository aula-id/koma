import type { RepoEntry } from '../../store/types/git'

export function repoOptionLabel(r: RepoEntry): string {
  const base = !r.rel || r.rel === '.' ? `${r.name} (workspace)` : r.rel
  return r.kind === 'worktree' || r.kind === 'submodule' ? `${base} (${r.kind})` : base
}
