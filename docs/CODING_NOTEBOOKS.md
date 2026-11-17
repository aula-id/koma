# Coding notebooks

Everyday `.ipynb` support in the Coding IDE. Document authority stays on the existing `coding.files` buffer (same dirty/save/fingerprint/recovery path as ordinary files). Tab kind remains `codingFile`; the router branches `.ipynb` to `NotebookTab`.

## Status

| Milestone | State |
|-----------|--------|
| M0 boundary | Shipped — no ordinary LSP / format-on-save / kernel on open |
| M1 cell editor | Shipped — lossless span JSON, cells + raw fallback, autosave |
| M2 analysis | **Unavailable** until `notebookDocumentSync` probe is green (`notebook-analysis.ts` + `coding::notebook_probe`) |
| M3 kernel | **Shell/IOPub + Select Kernel** — VS Code–style env picker (packs candidates), consent, secrets native-only |
| M4 outputs | Policy + DOMPurify HTML via inert sandboxed `srcdoc` iframe; stream/error/text/md/png/jpeg |

## Architecture

```text
NotebookTab
  └── NotebookController  (parse + history + epoch CAS)
        └── updateNotebookText → coding.files[key]   // sole saved-text authority
              └── saveCodingFile / formatBeforeSave bypass for .ipynb

M0 gates
  coding-disk.ts     no LspDidChange for notebooks
  store/push/coding.ts  no LspDidSave/Change after notebook save; Notebook* events → koma-notebook
  store/actions/tabs.ts no LspDidClose; dispose controller on force-close
  monaco-lsp.ts      notebook cell/raw schemes refused by modelToRootPath
  CodeEditorTab      defensive isNotebookPath skip on LspDidOpen

M3 native
  coding/notebook.rs         process ownership, consent, Windows JPY_INTERRUPT_EVENT, Unix PGID
  coding/notebook_wire.rs    multipart + HMAC-SHA256 (no key in events)
  coding/notebook_client.rs  DEALER shell/control/stdin + SUB iopub (zeromq pure Rust)
  launch only: {interpreter} -m ipykernel_launcher -f {connection_file}
  never metadata.kernelspec.argv
  connection file: private dir, owner-only, loopback, keys native-only
  has_active() includes live client threads
  consent(workspace, interpreter, path, documentEpoch) before start
```

## Frontend modules

| Path | Role |
|------|------|
| `lib/notebook-workspace.ts` | Host-bound buffer adapter + content/epoch CAS |
| `lib/notebook-document.ts` | Span-preserving JSON mutations (incl. `setCellOutputs`) |
| `lib/notebook-identity.ts` | Runtime cell ids (no dirty-on-open repair) |
| `lib/notebook-history.ts` | Structural undo stack |
| `lib/notebook-controller.ts` | Open-document controller registry |
| `lib/notebook-output-policy.ts` | Safe output classification (M4) |
| `lib/notebook-html.ts` | DOMPurify + srcdoc helper |
| `lib/notebook-analysis.ts` | M2 capability gate |
| `lib/notebook-runtime.ts` | Kernel RPC + IOPub → nbformat mapping |
| `lib/notebook-kernel-picker.ts` | Select Kernel: Python environment candidates (packs, project venvs), per-document remembered choice |
| `lib/coding-autosave.ts` | Shared 750 ms document autosave |
| `components/NotebookTab.tsx` | Cell UI + Run/Interrupt/Restart |
| `components/NotebookOutputs.tsx` | Shared disk + live output renderer |
| `components/NotebookCreateDialog.tsx` | New notebook dialog (`koma-new-notebook` event, default `Untitled.ipynb`) |

## Native RPC (Coding service)

Ops (camelCase on the wire): `notebookConsent`, `notebookKernelStart`, `notebookKernelStop`, `notebookKernelInterrupt`, `notebookKernelRestart`, `notebookKernelRelease`, `notebookKernelStatus`, `notebookIpykernelProbe`, `notebookExecute`.

Events (`CodingEvent.event.k`): `NotebookKernelState`, `NotebookIo`, `NotebookExecuteReply` — never include connection key/ports.

Hello capability token: `notebook`.

## Size policy

- Editable cell UI budget: **4 MiB** UTF-8 serialized notebook
- Oversized / invalid JSON: raw editor only; never silently strip outputs
- Host text read/save cap remains 5 MiB
- IOPub content capped (~256 KiB strings) before GUI events

## Security notes

- `metadata.trusted` grants nothing
- Notebook markdown preview must not resolve workspace images
- HTML outputs: DOMPurify allowlist → inert `iframe` with `sandbox=""` and `srcdoc` only (no `allow-scripts`, no remote `src`)
- Connection keys and ports never appear in frontend payloads or cell outputs
- Stdin / interactive input: v1 refuses (`allow_stdin: false` + empty input_reply)

## BasedPyright probe (M2)

```bash
KOMA_LSP_NOTEBOOK_PROBE=1 cargo test -p agent --no-default-features coding::notebook_probe -- --ignored --nocapture
```

Looks for the koma-managed `basedpyright-langserver` (under `~/.koma/lsp/basedpyright/`, resolved by `lsp::manifest::managed_binary_path`; override with `KOMA_BASED_PYRIGHT`).

**Last probe (dev host):** `notebookDocumentSync: true` on pinned basedpyright. Analysis UI still Unavailable until C3 client wiring (`notebookDocument/didOpen|didChange|didClose` + cell Monaco providers) ships — no half-enable, no cell-concat fallback.

## Verification

```bash
cd src-webgui && npm run test:logic -- src/lib/notebook
cargo test -p agent --no-default-features coding::notebook
cargo test -p agent --no-default-features coding::notebook_wire
KOMA_NOTEBOOK_KERNEL_TEST=1 cargo test -p agent --no-default-features coding::notebook::tests::live_kernel_execute_print -- --ignored --nocapture
KOMA_LSP_NOTEBOOK_PROBE=1 cargo test -p agent --no-default-features coding::notebook_probe -- --ignored --nocapture
cd src-webgui && npm run build
```
