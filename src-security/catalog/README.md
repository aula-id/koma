# Security tool catalog

Rows describe concrete toolkit entries. The model never receives thousands of
ToolDefs — only `sec_catalog` / `sec_run` / `sec_fs` / `sec_runtime`.

## Model surface (immutable)

| Mode | Advertised `sec_*` |
|------|-------------------|
| Security off | none |
| Agent / YOLO + daemon | `sec_catalog`, `sec_run`, `sec_fs`, `sec_runtime` |
| Plan + daemon | `sec_catalog` only |

Discovery ladder: categories → list (paged, lane-filtered, `filter=ready`) → get (one schema) → run.

## Working set (survives compaction)

Session file `sec_working_set.json` holds up to 16 pinned `{name, blurb, effect}` plus
`last_run`. Re-injected every turn in the volatile prompt tail. Fat catalog tool
results may be spilled/compacted; pins are rewritten from disk each turn.
Rediscovery is always available via `sec_catalog`.

## Enforcement on `sec_run`

name → catalog row → lanes → effect/lane matrix → scope if needed → ready → **guest only**
(browser may stay host until guest browser exists). Unknown → refuse.

## Network

Proxy rules deny-default. Empty rules = offline. Scope hosts/IPs+ports = allowlist.
Research should keep rules empty. Agent does not unilaterally open the internet.

## Packs / image

| Composition | Tag suffix | Role |
|-------------|------------|------|
| minimal | `-minimal` | daemon + proxy + smoke path |
| standard | (default `:version`) | power-user default CLIs |
| extended | `-extended` | large baked set |

Image name: `ghcr.io/aula-id/koma-safety`. Internal pack ids are not public SKUs
named web/red/full.

## Before adding 2300+ tools

1. Rows validate against `schema.json`.
2. `lanes` + `effect` + `exec` enforce; `categories` browse only.
3. Install via `install_recipes/` → bake into image; `ready` only when detect passes.
4. Taxonomy dumps (e.g. pentestreports) are not install recipes until resolved.
