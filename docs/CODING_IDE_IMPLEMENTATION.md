# Coding IDE implementation

The accepted scope extends the existing Monaco/LSP editor into a daily coding
environment. Workspace selection remains Koma's dropdown, with the existing
two-pane layout, visual tokens, and interaction patterns.

## Delivery ledger

| Milestone | State |
| --- | --- |
| Host/workspace identity and independent coding lifecycle | In progress |
| Document views, recovery, history, external changes | Pending |
| Language packs, toolchains, environment selection | Pending |
| Quick Open, commands, outline, navigation, settings | Pending |
| Workspace edits, formatting, rename, code actions, LSP extensions | Pending |
| Search/replace preview and task runner | Pending |
| Debug Adapter Protocol and debugger UI | Pending |
| Test Explorer and complete language catalogue | Pending |
| Cross-platform integration and native acceptance | Pending |

## Product decisions

- Every document, diagnostic, installation, and running process belongs to an
  explicit host/workspace. Changing dropdown, chat session, or host must not
  retarget existing work.
- Language support includes runtime/compiler installation. System prerequisites
  use curated OS installers with native elevation and a concrete install plan.
- Project configuration is Koma's own optional `.koma/coding.json`. No VS Code
  workspace/configuration import or extension-host compatibility is planned.
- Cover Rust, JS/TS, Python, Go, C/C++, PHP, Lua, Zig, Bash, Nix, JSON/HTML/CSS,
  and TOML. Capabilities distinguish unsupported platforms and non-applicable
  operations from missing installation. PHP retains Intelephense and offers
  Phpactor where supported; premium capabilities require the user's licence.
- Keep language-pack binaries shared per host/version and project environments
  independent. Tasks/debuggers/tests continue across UI context changes.
- Commit substantial changes independently. Static Rust/TypeScript checks are
  performed during implementation. Regression sources and native acceptance
  instructions are supplied; functional test execution belongs to the user.

## Required acceptance

Identical paths on separate hosts and multiple same-language roots must remain
isolated. Exercise typing during save/format, stale replies, failed writes,
external changes, recovery, cross-file undo, install interruption, environment
selection, debug attach/detach, discovery/run/cancel/rerun of tests, and SSH
reconnect. Native review covers Koma's Windows, Linux, and macOS targets. A
successful compile is not evidence of native workflow correctness.
