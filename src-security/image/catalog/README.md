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

name → catalog row → pack `lanes` (`gate_run`) → effect/lane matrix → scope if needed → ready → **guest only**
(browser may stay host until guest browser exists). Unknown pack row without lanes soft-admits legacy handlers.
Research/blue lazy `ensure_up` forces deny-all egress even if a prior red allowlist was open.
`sec_catalog list` defaults to `filter=ready`.

When a tool is not ready, `gate_run` / get include the **cyber-domain guest image**
(`image_or_pack`) and a hint to pull/activate it under `/security` → Images.

## Network

Proxy rules deny-default. Empty rules = offline. Scope hosts/IPs+ports = allowlist.
Research should keep rules empty. Agent does not unilaterally open the internet.

## Guest images (cyber domains, ≤9GB each)

| Composition | Tag suffix | Role |
|-------------|------------|------|
| **minimal** | `-minimal` (also bare `:version` / `:latest`) | Daemon plus handler binaries no domain image bakes |
| recon, recon-2, recon-ext, recon-src | `-recon…` | OSINT / enum / network |
| web, web-ext, web-src | `-web…` | App / API / CMS / vulns |
| identity, identity-ext, identity-src | `-identity…` | Auth / passwords / cracking |
| cloud, cloud-ext, cloud-src | `-cloud…` | Cloud services |
| rev, rev-ext, rev-src | `-rev…` | Reverse / binary / mobile |
| crypto, crypto-ext, crypto-src | `-crypto…` | Crypto / stego |
| pxn, pxn-ext, pxn-src | `-pxn…` | Host offensive / red / wireless (**not** `pwn`) |
| dfir, dfir-ext, dfir-src | `-dfir…` | Forensics / IR / defensive |
| lab, lab-ext, lab-src | `-lab…` | Dev / virt / misc lab |

Image name: `ghcr.io/aula-id/koma-safety`.

- `koma --security-install` pulls **minimal only**.
- Domain images: `/security` → **Images** pane (pull / activate / remove).
- No `lang-go` / `lang-cargo` SKUs — channel is bake detail; tags are cyber-space.
- Hard size cap **9 GB** per image (build script gate).
- Pack field `image_or_pack` = composition id.

See `install_recipes/COMPOSITIONS.md` and `install_recipes/buckets/`.

## Before adding more tools

1. Rows validate against `schema.json`.
2. `lanes` + `effect` + `exec` enforce; `categories` browse only.
3. Assign a domain composition; regenerate buckets via `gen_domain_buckets.py`.
4. Install channel lists under `buckets/`; `ready` when detect passes on that guest.
5. Taxonomy dumps are not install recipes until resolved.
