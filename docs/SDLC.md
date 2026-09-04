# SDLC mode

SDLC is Koma's opt-in delivery envelope. It stays inside `AgentMode::Sdlc` for
assess, approval, execute, integrate, and done. Reusing approval controls must
never hop the session into Plan or Auto.

**Speed split:** assess is intentionally slower (waterfall lock-in with the
user). After approve, implementation should feel as direct as Plan→approve→Auto
while hard rails stay on. Slowness belongs in requirements lock and evidence
depth — not in an extra post-y homework phase.

## Rails

| Rail | Authority |
|---|---|
| L1 contract | `<session>/mission.json`: goal, non-goals, acceptance, lane, verification, human gates, binding, and hash |
| L2 graph | `sdlc_nodes` and `sdlc_events` in the session SQLite log |
| L3 evidence | verification evidence, git history, artifacts, and recorded decisions |
| L0 chat | disposable working context; never the sole record of contract or completion |

The graph is authoritative. `/todo` and Explore project L2 `sdlc_nodes` while
in SDLC (not `TODO.md`). A node is sealed only by a passing `mission_verify`;
a checklist update cannot mark it done. The keeper reopens false-done leaves.

On approve the harness binds the mission worktree (cwd), advances to execute,
and auto-claims the first OPEN leaf when possible — the model should not
checkout, cd, or re-claim for the default path.

## Lifecycle

1. **Assess:** sequential lock-in with the user (goal → non-goals → acceptance →
   lane → gates → graph → branch/target). Workspace mutations are blocked. Use
   `mission_draft` to lock fields on the draft contract as answers arrive; chat
   is L0 only. `mission_ready` is blocked if a draft interview was started and
   required locks (goal, acceptance, lane, graph) are still unlocked. Full
   one-shot `mission_ready` remains valid when no draft locks were touched.
2. **Approve (y/a):** freeze the contract, bind a mission worktree and branch,
   then enter **execute** when bind succeeds (default). Prepare is optional —
   only for extra topology; `mission_prepare` is not required after a normal
   approve. Binding must be live before execute tooling matters.
3. **Execute:** main agent and subagents operate only in the bound worktree.
   One open leaf is claimed at a time. If a claim has `owned_paths`, writes stay
   inside those paths.
4. **Verify:** evidence is recorded per leaf. A passing verification seals the
   leaf and records the commit that produced the evidence.
5. **Integrate:** requires a valid binding, sealed evidence, a clean mission
   worktree (for merge), commits ahead of the frozen target, and satisfied human
   gates. **Lane ceremony:** `express` defaults to **branch-ready done** (mission
   branch left for PR/manual merge — no merge pressure; still evidence-gated).
   `standard` / `full` FF/merge into the frozen non-main target when clean; dirty
   target leaves the branch ready. `main`/`master` auto-merge stays blocked.
6. **Leave:** active missions are paused on disk; runtime phase is cleared and
   the prior mode and short-send setting are restored.

Same-batch trailing tools after `mission_ready` are skipped on approve/deny
(parity with Plan), so premature edit/bash in the approval turn do not run.

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
- SDLC does not hop to Auto after approve (execution stays in Sdlc under rails).
- SDLC does not auto-resume paused missions.
- SDLC does not create an assess worktree or operate dual assess worktrees.
- SDLC does not allow force-push, create a force-push allowlist, or rewrite the
  branch classifier.
- SDLC does not run a second autonomous keeper model.
- SDLC assess does not advertise or run MCP tools (fail-closed readonly).

## Hygiene

- `mission_clear` drops the TAC `approved_mission` stash (optional `reset` forces
  unapproved assess rails). Integrate→done also clears the stash.
- TAC mission bias applies only in prepare/execute/integrate phases.
- `mission_ready` parks with soft **BIND PREFLIGHT** warnings when primary is
  detached or target would be main/master; hard fail remains on approve bind.

## Verification matrix

The `agent` test suite covers phase persistence, contract reassessment,
claiming, ownership, push scope, keeper false-done handling, handoff authority,
verification evidence, integration safety, and assess branch restoration.
Run `cargo test -p agent`, `cargo clippy -p agent -- -D warnings`, and
`cargo fmt --check` before merging SDLC rail changes.
