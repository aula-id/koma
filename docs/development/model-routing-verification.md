# Model ownership and recovery verification

The regression suites cover scope-local ownership, legacy serialization and duplicate
normalization, TUI/GUI Main assignment and inheritance, Apple setup/toggle, live source
references, offline reference deletion, connection identity, final-only classifier verdicts,
truncation budgets, Main fallback, timeouts, approval cancellation, compaction preservation,
and Awareness generation/workspace checks.

Compile and lint with warnings treated as errors:

```sh
RUSTFLAGS='-D warnings' cargo check --workspace --all-targets
RUSTFLAGS='-D warnings' cargo clippy --workspace --all-targets -- -D warnings
cd src-webgui
./node_modules/.bin/tsc --noEmit
```

Runtime tests and visual review are intentionally left to user/CI. Run the full Rust
suite in an isolated user profile (existing tests use the application's data directory),
on Linux, Windows, and macOS. Do not skip failing tests.

Review these interactive flows in both TUI and GUI:

- Assign Main locally, change global Main, then inherit: the local choice survives
  the global edit; secondary roles survive inheritance and `/free` toggles.
- Edit a catalogue model/provider selected by a session; verify the displayed effective
  route changes. Delete it and reopen a saved session to verify its reference is released.
- With classifier providers unavailable, confirm exactly one approval for the pending
  call, no execution before approval, denial without execution, and rejection of stale
  GUI approval after cancellation. Plan and other tool gates must still apply.
- In `koma run`, unavailable classification returns a structured tool failure when no
  interactive controller can approve. Subagents preserve their existing classifier policy.
- Return null, whitespace, markup-only, error, or timeout from the compactor. Verify
  active and saved messages remain identical, the spinner clears, and no plan/mission
  continuation starts. A valid summary must preserve the requested tail.
- Deliver Awareness results in reverse order and switch workspaces while requests are
  pending. Only the latest generation for the active workspace may apply.

Apple empty responses are not presumed to be token exhaustion. Only `finish_reason=length`
plus empty/incomplete final JSON permits the one 2,000 → 4,000 retry. Actual Windows
provider behavior still needs verification from finish/usage diagnostics.
