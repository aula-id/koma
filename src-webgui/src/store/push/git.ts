import { receiveGitReply } from '../../lib/gitWorkbench'
import type { StoreGet, StoreSet } from '../api'
import { dedupCommits, updateBootstrap } from '../initial'
import type { PushEnvelope } from '../types/envelope'

export function pushGit(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'GitWorkbench': receiveGitReply(env); break

      case 'GitStatus':
        set((s) => {
          if (s.activeRepoRoot && env.root !== s.activeRepoRoot) return s
          return {
            git: {
            root: env.root,
            branch: env.branch,
            detached: env.detached,
            ahead: env.ahead,
            behind: env.behind,
            staged: env.staged,
            unstaged: env.unstaged,
            error: env.error,
            keyName: env.keyName,
            inProgress: env.inProgress,
            conflicted: env.conflicted,
            pushMode: env.pushMode ?? null,
            },
          }
        })
        break
      case 'GitDiff':
        set((s) => {
          const id = `gitdiff:${env.staged ? 'staged' : 'unstaged'}:${env.path}`
          // Ignore a reply for a tab closed while the req was in flight.
          if (!s.ui.tabs.some((t) => t.id === id)) return s
          return {
            ui: {
              ...s.ui,
              tabs: s.ui.tabs.map((t) =>
                t.id === id && t.kind === 'diff'
                  ? {
                      ...t,
                      loading: false,
                      diff: {
                        original: env.original,
                        modified: env.modified,
                        error: env.error,
                        binary: env.binary,
                        // GitDiff is always an actual git diff (never a
                        // non-git "virtual git" baseline) — unlike FileDiff,
                        // this reply has no `origin` field at all.
                        origin: 'git',
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'GitGraph':
        set((s) => {
          // Append (load-more) concatenates onto the existing page and dedupes;
          // replace (refresh / first load) drops the old page entirely. Only one
          // GitGraph request is ever in flight (see refreshGraph/loadMoreGraph),
          // so `loadMode` here is unambiguously the mode of THIS reply.
          const commits =
            s.graph.loadMode === 'append'
              ? dedupCommits([...s.graph.commits, ...env.commits])
              : env.commits
          return {
            graph: { ...s.graph, commits, head: env.head, hasMore: env.hasMore, loading: false },
          }
        })
        // A refreshGraph() that landed while this request was in flight couldn't
        // fire (serialization guard) and instead set pendingRefresh — now that
        // loading is committed false above, replay it.
        if (get().graph.pendingRefresh) {
          set((s) => ({ graph: { ...s.graph, pendingRefresh: false } }))
          get().refreshGraph()
        }
        break
      case 'CommitDetail':
        set((s) => {
          // Drop a stale reply for a since-changed selection (the echoed `sha` no
          // longer matches what's selected) — the detail pane keeps its loading
          // state until the reply matching the CURRENT selection lands.
          if (env.sha !== s.graph.selectedSha) return s
          return {
            graph: {
              ...s.graph,
              detail: {
                sha: env.sha,
                author: env.author,
                email: env.email,
                date: env.date,
                subject: env.subject,
                body: env.body,
                parents: env.parents,
                files: env.files,
                error: env.error,
              },
            },
          }
        })
        break
      case 'CommitDiff':
        set((s) => {
          const id = `commitdiff:${env.sha}:${env.path}`
          // Ignore a reply for a tab closed while the req was in flight.
          if (!s.ui.tabs.some((t) => t.id === id)) return s
          return {
            ui: {
              ...s.ui,
              tabs: s.ui.tabs.map((t) =>
                t.id === id && t.kind === 'diff'
                  ? {
                      ...t,
                      loading: false,
                      diff: {
                        original: env.original,
                        modified: env.modified,
                        error: env.error,
                        binary: env.binary,
                        // A commit diff is always a real git diff (`git show
                        // <sha>^1:…` vs `<sha>:…`) — never a "virtual git"
                        // baseline, so origin is always 'git'.
                        origin: 'git',
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'GitOp': {
        // Fetch/pull/push are the only ops that ever set `remoteBusy`; clearing
        // it unconditionally on every OTHER op is a harmless no-op (already null).
        const isRemote = env.op === 'fetch' || env.op === 'pull' || env.op === 'push'
        // Every op that can move HEAD or change the in-progress/conflict state
        // (branch-switcher/graph context menu ops G4 + the destructive/
        // interactive ops G5c: cherry-pick/revert/reset/merge/rebase, and the
        // conflict banner's abort/continue). All of these get a success toast
        // too — unlike the silent local mutations (stage/unstage/discard/
        // commit) — since HEAD/the branch list/conflict state just changed and
        // nothing else in the UI necessarily reflects that on its own.
        const HEAD_MOVING_OPS: Record<string, string> = {
          checkout: 'switched branch',
          createBranch: 'branch created',
          cherryPick: 'commit cherry-picked',
          revert: 'commit reverted',
          reset: 'branch reset',
          merge: 'merge complete',
          rebase: 'rebase complete',
          abort: 'operation aborted',
          continue: 'operation continued',
        }
        const isHeadMovingOp = env.op in HEAD_MOVING_OPS
        // Stash push/pop (GK4c) — neither moves HEAD, so they're kept OUT of
        // HEAD_MOVING_OPS (no graph refresh below), but still get a success
        // toast same as a head-moving op (nothing else in the UI reflects
        // "stashed"/"popped" on its own).
        const STASH_OPS: Record<string, string> = {
          stash: 'changes stashed',
          stashPop: 'stash applied',
        }
        const isStashOp = env.op in STASH_OPS
        // Surface a failed mutation the same de-duped way the Status case raises
        // a toast: only start a NEW toast when the text actually differs from
        // what's already showing, so a repeated failure (e.g. clicking "Stage
        // All" twice on a locked index) doesn't reset the auto-dismiss timer. A
        // SUCCESSFUL remote/head-moving op ALSO gets a toast — a short
        // confirmation using the host's own `message` if it sent one, else a
        // generic per-op label — so the outcome is visible even when nothing
        // else in the UI changes (e.g. a fetch with nothing new, or a
        // Continue that lands cleanly with no further conflicts). Local
        // mutations (stage/unstage/discard/commit) stay silent on success,
        // unchanged.
        const text = env.error
          ? `git ${env.op}: ${env.error}`
          : isRemote
            ? (env.message ?? `${env.op} complete`)
            : isHeadMovingOp
              ? HEAD_MOVING_OPS[env.op]
              : isStashOp
                ? STASH_OPS[env.op]
                : null
        const kind: 'error' | 'success' = env.error ? 'error' : 'success'
        set((s) => {
          const raise = !!text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return {
            ui: raise
              ? { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind } }
              : s.ui,
            // A successful commit empties the draft, ready for the next
            // message. Any other op — or a failed commit — leaves it alone
            // (a failed commit's typed message must not be lost).
            ...(env.op === 'commit' && env.ok ? { commitDraft: '' } : {}),
            // A remote op is no longer in flight once its reply lands, success
            // OR failure — clear the sync toolbar's busy flag.
            ...(isRemote ? { remoteBusy: null } : {}),
          }
        })
        // A successful HEAD-moving op (checkout/createBranch/cherryPick/revert/
        // reset/merge/rebase/abort/continue) moved HEAD, the branch list, and/or
        // the in-progress/conflict state — refresh the commit graph (its HEAD
        // ring) so it isn't left stale. NO explicit GitStatus refresh here: the
        // host ALREADY auto-follows every one of these ops with its own fresh
        // GitStatus push right after the mutation (git_host.rs's
        // spawn_{checkout,create_branch,cherry_pick,revert,reset,merge,rebase,
        // op_abort,op_continue}_attached each `push_git_op` then recompute +
        // send a fresh `GitStatusResult` unconditionally) — firing a SECOND
        // `GitStatus` request here would just be a redundant full-tree
        // `git status` scan racing the host's own, doubling status-panel
        // flicker/load with no benefit. The graph, unlike status, is NEVER
        // auto-pushed by the host, so it still needs this explicit refresh. A
        // conflicting cherry-pick/merge/rebase/etc. returns `ok:false` (git's
        // conflict exit IS reported as a failure here), so this gate skips the
        // graph refresh for it — that's fine, HEAD hasn't finalized on a
        // conflict, so there's nothing new for the graph to reflect until
        // `continue` succeeds. The conflict banner still appears regardless,
        // because the host pushes a fresh GitStatus unconditionally after
        // every op, independent of this `ok` gate.
        if (isHeadMovingOp) {
          // Refresh even after failure: checkout may have raced another worktree
          // or an active-repo change, and the host list is authoritative.
          get().refreshBranches()
        }
        if (isHeadMovingOp && env.ok) {
          get().refreshGraph()
        }
        // A stash push/pop changed the stash list either way (a failed pop
        // left it unchanged, but re-fetching is harmless) — refresh the
        // toolbar's Stash/Pop count. These ops never move HEAD, so — unlike
        // the branch above — this never triggers a graph refresh.
        if (isStashOp) {
          get().refreshStashes()
        }
        // A successful commit, fetch, pull, or push changes graph-visible refs
        // (new commits on HEAD, updated remote-tracking refs, or both) — refresh
        // the commit graph so it isn't left stale. Uses the existing serialized
        // refreshGraph() which guards against duplicate in-flight requests.
        // These ops are separate from HEAD-moving ops (checkout/merge/rebase etc.)
        // which are handled above, and from stash ops (no graph impact).
        if (env.ok && (env.op === 'commit' || env.op === 'fetch' || env.op === 'pull' || env.op === 'push')) {
          get().refreshGraph()
        }
        break
      }
      case 'KeyList':
        set(() => ({ keys: env.keys }))
        break
      case 'BranchList':
        // A list can race an active-repository switch. Never let a reply for the
        // old root repopulate branch controls for the new repository.
        set((s) => {
          const expectedRoot = s.activeRepoRoot ?? s.git.root
          // A stale generation (including one from the same root) must neither
          // replace data nor clear the loading state for the newer request.
          if (env.requestId != null && env.requestId !== s.branchesRequestId) return s
          if (env.root && expectedRoot && env.root !== expectedRoot) return s
          return { branches: env.branches, branchesLoading: false }
        })
        break
      case 'RepoList':
        set((s) => ({
          repos: env.repos,
          activeRepoRoot: env.active,
          ...(s.ui.bootstrap ? { ui: updateBootstrap(s.ui, { repos: 'done' }) } : {}),
        }))
        break
      case 'StashList':
        set(() => ({ stashes: env.entries }))
        break
      case 'Activity':
        set((s) => {
          // Drop a stale reply for a since-changed path filter (the echoed `path`
          // no longer matches what's currently requested) — lock-acquisition
          // order between two racing GitActivity requests isn't FIFO, so a
          // slower earlier reply can otherwise land after a faster later one and
          // clobber it. `null === null` still matches the whole-branch case.
          if (env.path !== s.activity.path) return s
          return {
            activity: { ...s.activity, commits: env.commits, error: env.error, loading: false },
          }
        })
        break
      case 'KeyReveal':
        set((s) => {
          // Toast only a COPY-public-key failure (`private: false`) — a
          // private-reveal failure already renders inline in the reveal box
          // (see SshKeysSettings' `revealedPrivate.error` branch) and would
          // otherwise double-surface. Same de-duped toast idiom as the KeyOp
          // case: only start a NEW toast when the text actually differs from
          // what's already showing.
          const text = env.error && !env.private ? `ssh key reveal: ${env.error}` : null
          const raise = !!text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return {
            keyRevealResult: {
              name: env.name,
              private: env.private,
              content: env.content,
              error: env.error,
            },
            ui: raise
              ? { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind: 'error' } }
              : s.ui,
          }
        })
        break
      case 'KeyOp':
        set((s) => {
          // Same de-duped toast idiom as the GitOp case above: only start a NEW
          // toast when the text actually differs from what's already showing.
          const text = env.error ? `ssh key ${env.op}: ${env.error}` : null
          const raise = !!text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          return raise
            ? { ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind: 'error' } } }
            : {}
        })
        break
    default:
      return false
  }
  return true
}
