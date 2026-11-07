# Security tool catalog

Rows describe concrete toolkit entries. The model never receives thousands of
ToolDefs — only `sec_catalog` / `sec_run` / `sec_fs` / `sec_runtime`.

## Before adding 2300+ tools

1. Every row MUST validate against `schema.json`.
2. `lanes` + `effect` + `exec` are enforcement; `categories` are browse-only.
3. Unknown / missing `lanes` → treat as red-only + network (fail closed) at the gate.
4. `plan_ok` is almost always `false` for concrete tools. Plan mode may call
   `sec_catalog` only.
5. Ship tools in packs (`packs/*.json`). `smoke` is the golden pack for the
   current built-in handlers.

## Packs

| Pack | Purpose |
|------|---------|
| `smoke` | Current ~28 daemon tools; E2E path for meta + boundary |
