# TUI /security

Three panes: Dashboard, Images, Refused. Tools are a catalog list, not a host-install grid.

## Dashboard

Gates are status, not arrow targets:

- runtime — container runtime up
- daemon — security daemon running
- guest — an image row is active, otherwise down
- boundary — guest boundary up
- lane `(l)` — session cyber lane

Buttons: Start/Stop `(s)` (never both), Runtime `(c)`, Refresh `(u)`.

YOLO strip: armed or locked, plus the agent mode label. Arm/Disarm `(o)`. Enter/Leave `(e)`. Enter stays dim until the daemon is running and YOLO is armed; the letter still works and refuses with `yolo locked — start the daemon and arm first`.

Guest map groups image rows by domain (recon, web, identity, cloud, rev, crypto, pxn, dfir, lab, then minimal). Columns are base, `-2` when that composition exists, ext, src. Glyphs: `██` active, `▒▒` local, `··` remote. Left/Right cycle map cells, including a leading all position that clears the filter.

Catalog list scrolls with Up/Down only. `f` cycles all / ready here / needs image. `/` searches name and blurb; Esc clears the query and leaves search without closing the panel. The selected strip shows ready (`●`), local (`▒`, Activate `(a)`), or remote (`·`, Pull `(p)`), plus image, exec, and the mark reason.

## Letters

```
s Start/Stop
c Runtime
u Refresh dashboard (catalog index + images). Not a daemon restart.
o Arm/Disarm
e Enter/Leave yolo
l Lane
h cycle · 1 2 3 jump panes
f filter · / search
p a x  pull / activate / remove
r Images refresh only
y n confirms only (no security confirm)
Space/Enter activate on Images; no-op on Dashboard and Refused
Esc closes, unless catalog search is active
```

`p` / `a` / `x` on Dashboard apply only when the selected tool's image is a real composition (not smoke/none/external/debian/empty) and it needs the op: pull when remote, activate when not ready, remove when local and not active. On Images they manage the selected composition.

## Images

Domain compositions (`recon`, `web`, `…`, `*-ext`, `*-src`) are pulled, activated, and removed here. `r` refreshes this list only.

## Refused

Handshake refusals (malformed or ungated descriptors). No restart button.

## Agent tools (security on + daemon)

- `sec_catalog` — categories / list (paged) / get
- `sec_run` — one concrete tool in the koma-safety guest
- `sec_fs` — copy_in / copy_out / ls
- `sec_runtime` — status | up | down | health | net | rules

Working-set pins survive chat compaction (session `sec_working_set.json`).
