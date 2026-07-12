# Architecture: Coding Panel

A lightweight workspace editor sidebar in koma's web GUI. Not an IDE — no LSP,
no language servers, no terminal. Browse, create, edit, save, rename, delete
workspace files. All changes tracked in Git via the existing Source Control panel.

## Scope

```text
Explorer → Coding
├── workspace file tree (from configured workdirs)
├── new file / new folder
├── editable Monaco file tabs
├── save / revert / dirty indicator
├── rename / delete with confirmation
└── file changed on disk detection (reload before save)
```

Explicitly **out of scope** for v1:

- Workspace-wide find/replace (v2)
- LSP, diagnostics, completion, formatting
- Terminal emulation
- File watcher (poll on tab activation instead)
- Merge/3-way conflict UI

## File Structure

### New Files (all < 600 lines)

```text
src-webgui/src/
├── lib/
│   └── monaco-setup.ts              ~120 lines  shared worker, theme, lang registry
│                                      (extracted from DiffTab.tsx)
├── components/
│   ├── panels/
│   │   └── CodingPanel.tsx           ~450 lines  file tree sidebar panel
│   └── CodeEditorTab.tsx             ~300 lines  editable Monaco tab
├── store/
│   └── coding.ts                     ~250 lines  file tree + file tab state + actions

src-agent/src/app/runtime/
├── client/
│   └── file_ops.rs                   ~350 lines  tree/read/save/create/delete/rename
```

### Modified Files

```text
src-webgui/src/
├── koma.d.ts                         +30 lines   FileTree/FileRead/FileSave/FileCreate/
│                                                  FileDelete/FileRename requests + push types
├── store/koma.ts                     +120 lines  Tab variant 'codingFile', merge coding
│                                                  slice, push cases
├── components/
│   ├── ActivityBar.tsx                +3 lines    { view:'coding', icon:Code2, label:'Coding' }
│   ├── Sidebar.tsx                    +8 lines    'coding' view + CodingPanel import
│   ├── TabBar.tsx                     +25 lines   file tab kind render branch
│   └── DiffTab.tsx                    -40 lines   extract shared setup → monaco-setup.ts
├── routes/
│   └── index.tsx                      +15 lines   file tab lazy import + routing

src-agent/src/app/runtime/gui/
├── proto.rs                          +35 lines   6 GuiReq variants + 6 HostCtl messages
├── dispatch.rs                       +60 lines   6 match arms → HostCtl::File*
└── ...

src-agent/src/app/runtime/client/
├── mod.rs                            +6 lines    HostCtl enum variants
├── host.rs                           +30 lines   match HostCtl::File* → file_ops
└── push_proto.rs                     +35 lines   6 push envelope variants
```

## Why These Splits

### monaco-setup.ts (~120 lines)

`DiffTab.tsx` currently inlines the worker, theme, and language registration.
Both `DiffTab` and `CodeEditorTab` need it. Extract once, import twice. Keeps
both tabs under 400 lines.

Shared exports:
- `initMonaco()` — register worker + languages (idempotent)
- `applyKomaTheme()` — resolve CSS custom properties → hex → Monaco theme
- `readMonoFont()` — read font from `document.body`
- `langFromPath()` — extension → language ID mapping

### coding.ts (~250 lines)

The main store (`koma.ts`) is already ~3600 lines. A separate slice keeps file
tree state, file tab dirty tracking, and coding actions isolated. Merges into
the main store via zustand slice merge pattern.

```ts
type CodingSlice = {
  roots: string[]               // configured workspace roots
  activeRoot: string | null     // currently selected root
  fileTree: Record<string, {    // keyed by root + path
    entries: FileTreeEntry[]
    loading: boolean
    error: string | null
  }>
  codingFiles: Record<string, { // keyed by root + normalized path
    content: string | null       // null = loading
    savedContent: string | null  // last saved content
    fingerprint: string          // disk fingerprint from last read/save
    dirty: boolean
    loading: boolean
    saving: boolean
    conflict: boolean            // stale save attempted
    error: string | null
    binary: boolean
    tooLarge: boolean
  }>
  openCodingTab: (root: string, path: string) => void
  saveCodingTab: (root: string, path: string) => void
  closeCodingTab: (root: string, path: string) => void
  revertCodingTab: (root: string, path: string) => void
  createFile: (root: string, path: string, kind: 'file' | 'dir') => void
  deleteFile: (root: string, path: string) => void
  renameFile: (root: string, oldPath: string, newPath: string) => void
  refreshTree: (root: string, path?: string) => void
  clearConflict: (root: string, path: string) => void
}
```

### CodingPanel.tsx (~450 lines)

The sidebar panel owns:
- Recursive file tree rendering with expand/collapse state
- Workspace root selector (from configured `workdir[]`)
- Context menu: new file, new folder, rename, delete
- Confirmation dialogs for destructive actions
- Click → `openFileTab(path)` to open in main editor area

Follows the pattern of `GitPanel.tsx` (~600+ lines) for panel structure.

### CodeEditorTab.tsx (~300 lines)

The editor tab owns:
- Standalone `monaco.editor.create()` (not diff editor)
- Model lifecycle: create on open, dispose on close, swap on content refresh
- Dirty tracking: `model.onDidChangeContent` → mark dirty
- Save: `req({ r: 'FileSave', root, path, content, expectedFingerprint })` → clear dirty
- Revert: `req({ r: 'FileRead', root, path })` → replace model content
- Stale detection: if `fingerprint` changed since read, prompt reload
- Conflict handling: display error when save returns conflict (stale fingerprint)
- Keyboard: Ctrl/Cmd+S intercepted → save, not browser save

### file_ops.rs (~350 lines)

All file operations in one host-local module. Each operation follows the
same pattern:

```text
1. Resolve path (containment check via workspace resolve)
2. Validate (not binary, not too large, within workspace)
3. Perform fs operation via tokio::fs
4. Return structured result
```

Operations:
- `handle_file_tree(root)` — recursive directory listing, skip `.git`/`node_modules`/`target`/`.koma`
- `handle_file_read(path)` — read text content + fingerprint (metadata + content hash)
- `handle_file_save(path, content, expected_fingerprint)` — compare fingerprint before writing; return conflict if mismatch
- `handle_file_create(path, is_dir)` — create file or directory
- `handle_file_delete(path)` — delete with safety checks
- `handle_file_rename(old, new)` — rename/move

Binary detection: read first 8KB, check for null bytes. Cap file reads at 5MB.
Return structured errors for binary/large/missing/permission cases.

## IPC Contract

### Requests (JS → Rust)

```ts
// Added to GuiReq union in koma.d.ts
{ r: 'FileTree'; root: string; path: string; requestId: string }
{ r: 'FileRead'; root: string; path: string; requestId: string }
{ r: 'FileSave'; root: string; path: string; content: string; expectedFingerprint: string; requestId: string }
{ r: 'FileCreate'; root: string; path: string; kind: 'file' | 'dir'; requestId: string }
{ r: 'FileRename'; root: string; oldPath: string; newPath: string; requestId: string }
{ r: 'FileDelete'; root: string; path: string; requestId: string }
```

### Pushes (Rust → JS)

```ts
// Added to PushEnvelope union (Rust) / CodingPush type (TS)
{ k: 'FileTree'; root: string; path: string; requestId: string; entries: FileTreeEntry[]; error: string | null }
{ k: 'FileRead'; root: string; path: string; requestId: string; content: string | null; fingerprint: string; binary: boolean; tooLarge: boolean; error: string | null }
{ k: 'FileSave'; root: string; path: string; requestId: string; fingerprint: string; error: string | null }
{ k: 'FileCreate'; root: string; path: string; requestId: string; error: string | null }
{ k: 'FileRename'; root: string; oldPath: string; newPath: string; requestId: string; error: string | null }
{ k: 'FileDelete'; root: string; path: string; requestId: string; error: string | null }
```

Every push echoes the workspace root, relative path, and request generation so
out-of-order or stale replies can be rejected by the frontend reducer.

### FileNode type

```ts
type FileNode = {
  name: string
  path: string           // workspace-relative
  isDir: boolean
  children?: FileNode[]  // only for expanded dirs (lazy-loaded on expand)
}
```

Tree loading is two-phase: `FileTree` returns top-level nodes; expanding a
directory sends `FileTree` with that directory's path as root. This avoids
sending the entire filesystem in one push.

## Data Flow

```text
CodingPanel                        CodeEditorTab
    │                                  │
    ├─ req FileTree ───────────────────┤
    ├─ req FileCreate                  ├─ req FileRead
    ├─ req FileDelete                  ├─ req FileSave
    ├─ req FileRename                  │
    │                                  │
    ▼                                  ▼
  HostCtl::File*                  HostCtl::File*
    │                                  │
    ▼                                  ▼
  file_ops.rs
  (resolve → contain → fs op → result)
    │                                  │
    ▼                                  ▼
  PushEnvelope::FileTree          PushEnvelope::FileReadResult / FileSaveResult
    │                                  │
    ▼                                  ▼
  CodingPanel reducer             CodeEditorTab reducer
  (tree state)                    (content, dirty, diskVersion)
```

## Tab Identity

File tabs use the `codingFile` kind with a path-derived stable ID:

```ts
| { id: string; kind: 'codingFile'; root: string; path: string; title: string }
```

- `id` = `file:<root>:<path>` (workspace-root + relative path, stable for dedup)
- `root` is the absolute workspace root (from configured `workdir[]`)
- Clicking the same file focuses the existing tab instead of opening a duplicate
- Dirty indicator shown in TabBar via `●` prefix on title
- Close with unsaved changes requires a floating dirty-close popover (not `window.confirm`).
- Tab id uses both root and path: `file:<root>:<path>`

## Modified Tab Union

```ts
export type Tab =
  // ... existing variants ...
  | { id: string; kind: 'codingFile'; root: string; path: string; title: string }
```

## ActivityBar / Sidebar Wiring

```ts
// ActivityBar.tsx — ACTIVITY_BAR_ITEMS
{ view: 'coding', icon: Code2, label: 'Coding' }

// Sidebar.tsx — SidebarView type
'explore' | 'git' | 'coding' | 'mcp' | 'connector' | 'agents' | 'usage' | 'store'

// Sidebar.tsx — TITLES
{ coding: 'Coding' }

// Sidebar.tsx — render
{view === 'coding' && <CodingPanel />}
```

## Rust Routing

```proto
// proto.rs — GuiReq enum additions
FileTree { root: String, path: String, request_id: String },
FileRead { root: String, path: String, request_id: String },
FileSave { root: String, path: String, content: String, expected_fingerprint: String, request_id: String },
FileCreate { root: String, path: String, kind: String, request_id: String },
FileRename { root: String, old_path: String, new_path: String, request_id: String },
FileDelete { root: String, path: String, request_id: String },
```

```rust
// dispatch.rs — handle_gui_req match arms
GuiReq::FileTree { root, path, request_id } => ctx.ctl.send(HostCtl::FileTree { root, path, request_id }),
GuiReq::FileRead { root, path, request_id } => ctx.ctl.send(HostCtl::FileRead { root, path, request_id }),
GuiReq::FileSave { root, path, content, expected_fingerprint, request_id } => ctx.ctl.send(HostCtl::FileSave { root, path, content, expected_fingerprint, request_id }),
GuiReq::FileCreate { root, path, kind, request_id } => ctx.ctl.send(HostCtl::FileCreate { root, path, kind, request_id }),
GuiReq::FileRename { root, old_path, new_path, request_id } => ctx.ctl.send(HostCtl::FileRename { root, old_path, new_path, request_id }),
GuiReq::FileDelete { root, path, request_id } => ctx.ctl.send(HostCtl::FileDelete { root, path, request_id }),
```

```rust
// client/mod.rs — HostCtl enum additions
FileTree { root: String, path: String, request_id: String },
FileRead { root: String, path: String, request_id: String },
FileSave { root: String, path: String, content: String, expected_fingerprint: String, request_id: String },
FileCreate { root: String, path: String, kind: String, request_id: String },
FileRename { root: String, old_path: String, new_path: String, request_id: String },
FileDelete { root: String, path: String, request_id: String },
```

## Security

- All paths resolved and checked for workspace containment before any fs operation.
- Binary files rejected on read (null-byte detection in first 8KB).
- File reads capped at 5MB.
- `target/`, `node_modules/`, `.git/`, `.koma/` excluded from tree listings.
- Delete requires explicit user confirmation in the UI.
- No automatic staging or committing — Git operations remain in the Source Control panel.
- Save is explicit by default (Ctrl/Cmd+S or Save button). Optional per-session
  `coding_autosave` enables a 750ms debounced auto-save in the GUI Coding panel.

## Build Verification

```bash
# Frontend
cd src-webgui && npm run build

# Rust
cargo test
cargo build -p agent --features gui
```
