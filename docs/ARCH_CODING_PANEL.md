# Architecture: Coding Panel

Workspace file editor in koma’s web GUI (Coding activity-bar view). Monaco tabs plus a multi-root tree; **host-spawned language servers** power completion, hover, go-to-definition, references, symbols, and diagnostics. A separate **Terminal** tab kind lives in the main area (not inside this sidebar).

## Scope

```text
ActivityBar → Coding sidebar
├── workspace file tree (configured workdirs)
├── new file / new folder / rename / delete
├── editable Monaco codingFile tabs (split panes)
├── save / revert / dirty / conflict (fingerprint)
├── host LSP (didOpen / Change / Save / Close + requests)
├── Problems drawer (publishDiagnostics)
└── Language Servers footer drawer (runtime status)
```

Related chrome (not CodingPanel itself):

| Feature | Location |
|---------|----------|
| Terminal tabs | `TerminalTab.tsx`, tab `kind: 'terminal'` |
| Language server install UI | Settings → Language servers; CLI `koma lsp …` |
| Diff tabs | Explore / Git file-changed → Monaco **diff** (Monarch only; no LSP on that path) |

Still **out of scope**:

- Workspace-wide find/replace across the whole tree
- Merge / 3-way conflict UI
- Terminal *embedded inside* the Coding sidebar (use Terminal tabs)

## File structure (current)

```text
src-webgui/src/
├── lib/
│   ├── monaco-setup.ts       shared Monaco worker / theme / Monarch langs
│   ├── monaco-lsp.ts         Monaco ↔ host LSP providers
│   └── lsp-bridge.ts         pending request map + URI helpers
├── components/
│   ├── panels/CodingPanel.tsx
│   ├── CodeEditorTab.tsx     editable Monaco + LSP attach
│   ├── ProblemsDrawer.tsx
│   ├── LspDrawer.tsx
│   └── TerminalTab.tsx       sibling feature
└── store/koma.ts             coding + tabs + LSP push handlers

src-agent/src/
├── lsp/                      catalog, install, resolve, JSON-RPC client
└── app/runtime/client/
    ├── file_ops.rs           tree / read / save / create / delete / rename
    └── lsp_host.rs           install/status + drain LSP replies → GUI
```

## Code editor + LSP

`CodeEditorTab` owns the Monaco model lifecycle (open / save / dirty / conflict). When a matching server is installed, it attaches providers via `monaco-lsp` / `lsp-bridge`.

- **Host** spawns servers (`src-agent/src/lsp/client.rs`), not Monaco language-server workers.
- Document sync: `textDocument/didOpen|didChange|didSave|didClose`.
- Requests: completion, hover, definition, references, documentSymbol.
- Diagnostics: `textDocument/publishDiagnostics` → Problems drawer + markers.
- Runtime rows: Language Servers footer drawer (`LspRuntime` pushes).
- Install root: `~/.koma/lsp/` (`koma lsp status|install|uninstall …`).
- Catalogue: vtsls, vscode-langservers, bash-language-server, intelephense, taplo, …; some entries PATH-only (e.g. lua-ls, zls, nil).

Diff editor path stays Monarch-only (syntax highlighting without host LSP).

## File ops

`file_ops.rs` (host-local):

1. Resolve path (workspace containment)
2. Validate (not binary / too large / outside root)
3. FS op
4. Structured result push

Ops: tree (lazy dirs; skip `.git` / `node_modules` / `target` / `.koma`), read + fingerprint, save with expected fingerprint (conflict on mismatch), create file/dir, delete, rename.

Binary: null-byte check on first 8KB. Read size cap applies.

## IPC (file ops)

**Requests (JS → host):** `FileTree`, `FileRead`, `FileSave`, `FileCreate`, `FileRename`, `FileDelete` (each with `root`, path fields, `requestId`).

**Pushes:** matching `File*` envelopes echoing root/path/`requestId` so the reducer can drop stale replies.

## IPC (LSP)

GuiReq / HostCtl surface document sync + completion/hover/definition/references/documentSymbol with `request_id`; pushes include diagnostics, completion/hover/… replies, install/status, and live runtime rows. See `gui/proto.rs`, `client/mod.rs` HostCtl, `push_proto.rs`, `lsp_host.rs`.

## Tab identity

```ts
| { id: string; kind: 'codingFile'; root: string; path: string; title: string }
// id = file:<root>:<path>  — stable for dedup; dirty ● on title
```

Other main-area kinds include `diff`, `terminal`, `settings`, extension panels, etc.

## ActivityBar / Sidebar

Built-in `ACTIVITY_BAR_ITEMS` / `SidebarView` include:

`explore` · `coding` · `git` · `mcp` · `connector` · `importGraph` · `agents` · `usage` · `store` · `remote`

Settings / Help / Tutorial are pinned footer chrome (not `SidebarView` values). Extension panel icons can appear on the bar via the same order/hidden machinery.

## Security

- Containment-checked paths before any FS op
- Binary / size gates; noisy dirs skipped in tree
- Delete needs UI confirmation; Git stays in Source Control
- Save: Ctrl/Cmd+S or optional session `coding_autosave` debounce
- LSP binaries are koma-managed or PATH-discovered — **not** marketplace extensions

## Build verification

```bash
cd src-webgui && npm run build
cargo test -p agent
cargo build -p agent --features gui
```
