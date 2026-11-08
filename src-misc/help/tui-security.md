# TUI /security

Isolation and security-runtime dashboard (not a per-tool enable list).

## Controls
- **Daemon** — start/stop host security control plane
- **YOLO arm** — unlocks `/mode yolo` (arm ≠ mode). Requires running daemon.
- **Runtime / boundary** — docker readiness, guest up/down, egress rules summary
- **Catalog summary** — domain counts; concrete tools via agent `sec_catalog`

## UI (OpenTUI)
- **Clickable pane tabs:** Dashboard · Images · Deps · Refused
- **Scrollable lists** with rail when long (Images / Deps / Refused)
- **Progress meters:** catalog ready, deps present, guest images local, refused
- **Clickable action rows** per pane (Activate / Pull / Install / Runtime…)
- Keyboard shortcuts still work as backup; footer controls are mouse-friendly

## Agent tools (security on + daemon)
- `sec_catalog` — categories / list (paged) / get
- `sec_run` — one concrete tool in koma-safety guest
- `sec_fs` — copy_in / copy_out / ls
- `sec_runtime` — status | up | down | health | net | rules

Working-set pins survive chat compaction (session `sec_working_set.json`).

## Path
1. `koma --security-install` once (pulls **minimal** guest only)
2. `/security` start daemon
3. Scope + lane
4. Optional YOLO arm → `/mode yolo`
5. catalog → run inside guest

## Images
- Domain compositions (`recon`, `web`, `…`, `*-ext`, `*-src`) are pulled/activated from the `/security` **Images** pane — not by `--security-install`.
- Install writes `~/.koma/security/images.json` and pins active `minimal`.

Image: `ghcr.io/aula-id/koma-safety` (`:version-minimal` / bare `:version` = basic; domain suffixes for power users).
