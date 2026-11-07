# TUI /security

Isolation and security-runtime dashboard (not a per-tool enable list).

## What it controls
- **Daemon** — start/stop the host security control plane
- **YOLO arm** — unlocks `/mode yolo` (arm ≠ mode). Requires a running daemon.
  After arming, switch with `/mode yolo` or Shift+Tab for hands-off runs.
- **Runtime strip** — docker/colima readiness, lane, refused descriptors
- **Catalog summary** — how many toolkit rows are loaded (by domain). Concrete
  tools are discovered dynamically by the agent via `sec_catalog` / `sec_run`.

## Agent tools (when security is on)
- `sec_catalog` — browse categories / list / get (Plan may use this only)
- `sec_run` — execute one concrete tool (lane + scope gates)
- `sec_fs` — copy files into/out of the guest
- `sec_runtime` — boundary status / up / down / health

## Typical path
1. `koma --security-install` (once)
2. `/security` → start daemon
3. Confirm scope + pick lane (research / red / blue)
4. Optional: arm YOLO, then `/mode yolo`
5. Agent uses catalog → run inside the guest

Use /help for the registered command and keyboard reference.
