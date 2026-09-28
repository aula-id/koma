// One file entry in a GitStatus's staged/unstaged list — mirrors the host's
// `GitFileEntry` (git.rs, `rename_all = "camelCase"`). `status` is a single
// git-porcelain status character ("M"/"A"/"D"/"R"/"C"/"U"/"?") the GIT panel
// renders as a badge. `origPath` is non-null only for a rename/copy record
// (shown as `origPath -> path`). A single on-disk path can legitimately
// appear in BOTH `staged` and `unstaged` (e.g. `MM`) — not a bug.
export type GitFileEntry = {
  path: string
  origPath: string | null
  status: string
  staged: boolean
}

// The Source Control "GIT" panel's authoritative status (host `GitStatus`
// push, mirrors `GitStatusResult` verbatim). `error` set means the working
// directory isn't a git repository (or `git status` failed) — every other
// field then sits at its neutral default. Global (not per-session) in the
// store, mirroring how the host resolves it off the foreground session's
// workdir; refreshed via `refreshGitStatus()`.
export type GitStatus = {
  root: string | null
  branch: string | null
  detached: boolean
  ahead: number | null
  behind: number | null
  staged: GitFileEntry[]
  unstaged: GitFileEntry[]
  error: string | null
  // The SSH vault key (by name) currently assigned to this repo for remote ops
  // (wave 4b), or null when none is assigned (remote ops then use the system
  // default ssh-agent/keys). Mirrors the host's `GitStatusResult.key_name`. The
  // GIT panel's key picker's current selection.
  keyName: string | null
  // Which sequencer op (if any) is currently mid-flight (G5c) — one of
  // "merge"/"cherry-pick"/"revert"/"rebase", or null when the repo is clean.
  // Mirrors the host's `GitStatusResult.in_progress`. Drives the ConflictBanner
  // (Abort/Continue).
  inProgress: string | null
  // The porcelain-v2 unmerged/conflict file records (G5c), split out of
  // `staged`/`unstaged` — a conflicted file shouldn't masquerade as an ordinary
  // modification. Empty outside a conflict. Mirrors the host's
  // `GitStatusResult.conflicted`.
  conflicted: GitFileEntry[]
  pushMode: GitPushMode | null
}

export type GitPushMode = 'automatic' | 'plain' | 'set-upstream' | 'force-with-lease'

// ---- Commit graph (G2) wire types — mirror the host's git_graph.rs DTOs
// (every struct `rename_all = "camelCase"`), matched field-for-field so a wire
// mismatch can't silently read `undefined` at runtime. -----------------------

// One branch entry in a BranchList reply — host `BranchInfo` (git_branch.rs,
// `rename_all = "camelCase"`) — G4. `kind` is "local" (`refs/heads/…`),
// "remote" (`refs/remotes/…`, e.g. `origin/main`), or "tag" (`refs/tags/…` —
// GK4a, listed alongside branches for the React ref-tree); `isCurrent` marks
// the single branch HEAD currently points at (never true for a remote or tag
// entry).
export type BranchInfo = {
  name: string
  kind: 'local' | 'remote' | 'tag'
  isCurrent: boolean
  worktreePath?: string
}

// One repo entry in a RepoList reply — host `RepoEntry` (multi-repo support,
// `rename_all = "camelCase"`). `root` is the repo's absolute workdir root
// (also the `SetActiveRepo` request's `root` value); `name` is its display
// label (basename) for the repo picker.
export type RepoEntry = { root: string; name: string }

// One stash entry in a StashList reply — host `StashEntry` (git_stash.rs,
// `rename_all = "camelCase"`) — GK4c. `index` is the `stash@{N}` slot number
// (0 is the most-recently-pushed stash); `message` is everything after the
// `stash@{N}: ` marker verbatim (covers both git's default "WIP on <branch>:
// …" message and a custom `git stash push -m <msg>`).
export type StashEntry = {
  index: number
  message: string
}

// One ref (branch/tag/HEAD pointer) decorating a commit — host `GitRef`. `kind`
// classifies it off the FULL ref path host-side (a distinct chip colour per
// kind); `isHead` marks the single `HEAD -> …` current-branch pointer.
export type GitRef = {
  name: string
  kind: 'head' | 'local' | 'remote' | 'tag'
  isHead: boolean
}

// One commit row in a GitGraph reply — host `GitCommitNode`. The list is
// newest-first, already `--date-order --parents`; `parents` is empty for a root
// commit; `refs` is empty for the common case of a commit nothing points at.
export type GitCommitNode = {
  sha: string
  parents: string[]
  refs: GitRef[]
  author: string
  email: string
  date: string
  subject: string
}

// One commit row in an Activity reply (GK5b) — host `ActivityCommit`. `date`
// is the author date as an ISO-8601 string (parsed client-side, never
// host-side); `added`/`deleted` are the commit's SUMMED line counts across
// every changed file (binary files contribute 0 to both).
export type ActivityCommit = {
  sha: string
  author: string
  email: string
  date: string
  added: number
  deleted: number
}

// One changed-file entry in a CommitDetail — host `CommitFile`. `status` is
// git's own token ("M"/"A"/"D"/"R100"/…); `origPath` is non-null only on a
// rename/copy record (then `path` is the NEW path, `origPath` the OLD one).
export type CommitFile = {
  status: string
  path: string
  origPath: string | null
}

// A single commit's full metadata (incl. body) + first-parent changed-file list
// — host `CommitDetailResult`. `error` non-null means the sha failed validation
// or the workdir isn't a git repo (every other field then a neutral default).
// Stored into the graph slice's `detail` by the CommitDetail push.
export type CommitDetail = {
  sha: string
  author: string
  email: string
  date: string
  subject: string
  body: string
  parents: string[]
  files: CommitFile[]
  error: string | null
}

// One keypair entry in the Settings "SSH Keys" section's vault list — mirrors
// the host's `KeyInfo` (keys.rs, `rename_all = "camelCase"`). This is a
// GUI-only, manual, user-owned key vault (`<~/.koma>/keys/`), completely
// separate from the model's own git credential machinery.
export type KeyInfo = {
  name: string
  fingerprint: string
  comment: string
  keyType: string
}

// A one-shot reveal of a keypair's contents (KeyReveal push) — the SSH Keys
// section's transient "Copy public key" / "Reveal private key" result, kept
// separate from the authoritative `keys` list. `private` echoes which half
// was read; `error` set means `content` is empty.
export type KeyReveal = {
  name: string
  private: boolean
  content: string
  error: string | null
}

