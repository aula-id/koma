# SDLC mode

SDLC is Koma's opt-in, away-from-desk delivery envelope. It is intentionally
slower than Auto and keeps the agent inside `AgentMode::Sdlc` for assess,
approval, execute, integrate, and done. Reusing approval controls must never
hop the session into Plan or Auto.

## Rails

| Rail | Authority |
|---|---|
| L1 contract | `<session>/mission.json`: goal, non-goals, acceptance, lane, verification, human gates, binding, and hash |
| L2 graph | `sdlc_nodes` and `sdlc_events` in the session SQLite log |
| L3 evidence | verification evidence, git history, artifacts, and recorded decisions |
| L0 chat | disposable working context; never the sole record of contract or completion |

The graph is authoritative. `TODO.md` is a projection. A node is sealed only
by a passing `mission_verify`; a checklist update cannot mark it done. The
keeper reopens false-done leaves.

## Lifecycle

1. **Assess:** establish the mission, graph, verification plan, human gates, and
   branch/worktree intent. Workspace mutations are blocked.
2. **Approve:** freeze the contract and bind a mission worktree and branch.
   Binding must be live before execute is enabled.
3. **Execute:** the main agent and subagents operate only in the bound
   worktree. One open leaf is claimed at a time. If a claim has `owned_paths`,
   writes stay inside those paths.
4. **Verify:** evidence is recorded per leaf. A passing verification seals the
   leaf and records the commit that produced the evidence.
5. **Integrate:** requires a valid binding, sealed evidence, a clean mission
   worktree, commits ahead of the frozen target, and satisfied human gates. A
   dirty target leaves the mission branch ready rather than disturbing user WIP.
6. **Leave:** active missions are paused on disk; runtime phase is cleared and
   the prior mode and short-send setting are restored.

## Authority boundaries

In-scope progress may be recorded automatically. A structured subagent handoff
may add notes, artifacts, evidence references, decisions, blocked status, or
children below its claimed leaf. It cannot seal work, expand ownership, or
change mission goal, acceptance, non-goals, lane, verification plan, human
gates, branch, worktree, or frozen target. Those contract-edge changes require
reassessment and reapproval.

The keeper automatically reopens false-done leaves. An invalid contract,
graph hash, or binding moves the session back to reassessment and disables
execute tooling; this is a rail, not merely a prompt.

## Git rules

- Never force-push.
- During execute/integrate, plain `push` may target only the mission branch and
  requires an explicit refspec.
- SDLC never auto-commits.
- Verification evidence is tied to an existing mission-branch commit.
- Integration never overwrites dirty target work. It either safely integrates
  into the frozen target or leaves the branch ready for human review.

## Explicit non-goals

- SDLC does not change Auto, Normal, Plan, or Yolo behavior.
- SDLC does not auto-resume paused missions.
- SDLC does not create an assess worktree or operate dual assess worktrees.
- SDLC does not allow force-push, create a force-push allowlist, or rewrite the
  branch classifier.
- SDLC does not run a second autonomous keeper model.

## Verification matrix

The `agent` test suite covers phase persistence, contract reassessment,
claiming, ownership, push scope, keeper false-done handling, handoff authority,
verification evidence, integration safety, and assess branch restoration.
Run `cargo test -p agent`, `cargo clippy -p agent -- -D warnings`, and
`cargo fmt --check` before merging SDLC rail changes.
