# TUI /security

Isolation and security-runtime dashboard (not a per-tool enable list).

## Controls
- **Daemon** — start/stop host security control plane
- **YOLO arm** — unlocks `/mode yolo` (arm ≠ mode). Requires running daemon.
- **Runtime / boundary** — docker readiness, guest up/down, egress rules summary
- **Catalog summary** — domain counts; concrete tools via agent `sec_catalog`

## Agent tools (security on + daemon)
- `sec_catalog` — categories / list (paged) / get
- `sec_run` — one concrete tool in koma-safety guest
- `sec_fs` — copy_in / copy_out / ls
- `sec_runtime` — status | up | down | health | net | rules

Working-set pins survive chat compaction (session `sec_working_set.json`).

## Path
1. `koma --security-install` once  
2. `/security` start daemon  
3. Scope + lane  
4. Optional YOLO arm → `/mode yolo`  
5. catalog → run inside guest  

Image: `ghcr.io/aula-id/koma-safety` (`minimal` / `standard` / `extended` compositions).
