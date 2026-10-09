import type { GitToolTab } from '../../lib/gitWorkbench'
import type { DiffPayload } from './session'

// One editor tab over the main content column. tabs[0] is ALWAYS the permanent,
// uncloseable chat tab; diff tabs are opened from the Explorer's File-changed
// rows. The `kind` discriminant is deliberately left open — a future
// `{ kind: 'session' }` variant (multi-session tabs, deferred but planned) slots
// in additively without disturbing existing consumers.
export type Tab =
  | GitToolTab
  | { id: 'chat'; kind: 'chat' }
  // The singleton Settings page (VSCode-style), opened from the ActivityBar gear.
  // Deduped by the fixed id 'settings'; closeable like a diff tab.
  | { id: 'settings'; kind: 'settings' }
  // The singleton Help page — a static, wire-free reference for the GUI's own
  // features (composer/sessions/tabs/keyboard). Opened from the ActivityBar's
  // (?) button, directly above Settings. Deduped by the fixed id 'help';
  // closeable like a diff tab. Mirrors the Settings tab's plumbing exactly.
  | { id: 'help'; kind: 'help' }
  // Singleton Tutorial coach tab (NLP + driver.js). ActivityBar above Help.
  | { id: 'tutorial'; kind: 'tutorial' }
  | {
      // Stable id — `diff:${path}` for a File-changed diff (find-by-path is
      // trivial), or `gitdiff:${staged ? 'staged' : 'unstaged'}:${path}` for a
      // GIT-panel diff (a git diff needs BOTH a staged and unstaged tab for
      // the SAME path open side by side, so `diff:${path}` alone would
      // collide — the `staged`/`unstaged` segment disambiguates).
      id: string
      kind: 'diff'
      // The path exactly as the fileChanges record (or GitFileEntry) carries
      // it — the key for the FileDiff/GitDiff req + reply.
      path: string
      // Basename of `path`. TabBar adds a dim parent-dir suffix at render time
      // when two open tabs share a basename (collision depends on the live tab
      // set, so it's resolved there, not baked into the stored title).
      title: string
      // Filled by the FileDiff/GitDiff reply; undefined until the first reply
      // lands.
      diff?: DiffPayload
      // True while a FileDiff/GitDiff req is in flight (initial open OR a
      // re-request on re-activate). A stale `diff` keeps rendering while
      // loading so re-focus never flashes to a spinner.
      loading: boolean
      // Present ONLY on a GIT-panel diff tab (opened via `openGitDiffTab`):
      // `true` = staged (index vs HEAD), `false` = unstaged (worktree vs
      // index). Undefined on a plain File-changed diff tab — this is what
      // `activateTab`'s re-request routes on (GitDiff vs FileDiff).
      staged?: boolean
      // Present ONLY on a commit-graph diff tab (opened via `openCommitDiffTab`):
      // the commit sha whose first-parent diff this tab shows. Distinct tab-id
      // scheme (`commitdiff:${sha}:${path}`) from the File-changed (`diff:`) and
      // GIT-panel (`gitdiff:`) schemes, so a commit-history diff never collides
      // with either. `activateTab`'s re-request checks this FIRST (GitCommitDiff)
      // before the `staged` (GitDiff) / plain (FileDiff) branches — a commit-diff
      // tab has no `staged`, so it would otherwise wrongly re-fire FileDiff.
      commitSha?: string
    }
  // A read-only STREAM tab live-streaming ONE sub-agent's transcript. Stable id
  // `sa:${agentId}` so open/dedupe is trivial. Content is NOT stored on the tab — the
  // StreamTab reads the live entry from `session.subagents` by `agentId` (so it updates
  // as the host pushes fresh transcript). `title` is the agent name at open time.
  | { id: string; kind: 'subagent'; agentId: number; title: string }
  // A read-only STREAM tab live-streaming ONE bash job's output. Stable id
  // `bash:${jobId}`; content read live from `session.bash` by `jobId`. `title` is the
  // (truncated) command.
  | { id: string; kind: 'bash'; jobId: number; title: string }
  // Per-agent editor tab (Agents sidebar panel). `agentId` is the agent's
  // NAME for an edit, `null` for a create — NOT the settings/help singleton
  // pattern: open-or-focus is keyed PER agentId (two different agents' editors
  // can be open side-by-side; re-clicking the same agent's row just focuses
  // its existing tab), matched by `agentId`, NOT by `id`. `id` is a client-
  // minted, STABLE identifier independent of `agentId` — deliberately, so a
  // successful create/rename (which mutates `agentId` via `renameAgentTab`)
  // never changes this tab's React `key` (`key={t.id}` in TabbedMain) and
  // never forces a remount that would wipe in-progress edits held in the
  // component's local state right as Save fires. Closeable like a diff tab;
  // closing an unsaved tab discards silently (no local draft is ever
  // persisted to the store).
  | { id: string; kind: 'agent'; agentId: string | null }
  // Lazy Skills editor; null skillId denotes Create.
  | { id: string; kind: 'skill'; skillId: string | null; title: string }
  | { id: 'upload-skill'; kind: 'uploadSkill' }
  // Installed-extension detail tab (Tab-B) — full manifest projection for one
  // locally-installed extension. Deduped by `extId`; closeable like a diff tab.
  | { id: string; kind: 'installedExtension'; extId: string; title: string }
  // The singleton GitKraken-style commit-graph tab (id 'graph'), opened from the
  // Source Control panel header. Deduped by the fixed id; closeable like a diff
  // tab. Content (commits/selection/detail) lives in the `graph` store slice, not
  // on the tab — the GraphTab reads it live and fires refreshGraph on mount.
  | { id: 'graph'; kind: 'graph' }
  // The singleton import-graph tab (id 'import-graph'), opened from the Import
  // Graph sidebar panel. Content (nodes/edges/selection) lives in the
  // `importGraph` store slice — the ImportGraphTab reads it live and fires
  // refreshImportGraph on mount.
  | { id: 'import-graph'; kind: 'importGraph' }
  // The singleton Analytics dashboard tab (id 'analytics'), opened from the
  // Usage sidebar's pinned "See Analytics" footer. Deduped by the fixed id;
  // closeable like a graph tab. Content lives in the `analytics` store slice.
  | { id: 'analytics'; kind: 'analytics' }
  // The singleton extension-STORE tab (id 'store'), opened from the ActivityBar's
  // Store icon / its Sidebar panel. Deduped by the fixed id; closeable like a diff
  // tab. Content (catalogue/detail/installed) lives in the `store` slice — the
  // StoreTab reads it live and fires browseStore + refreshInstalled on mount.
  | { id: 'store'; kind: 'store' }
  // One extension-contributed PANEL tab, opened from its merged ActivityBar icon
  // (see resolveActivityBarOrder's extension-item merge). Stable id
  // `ext:${extId}:${panelId}` — singleton PER PANEL (not per-extension: an
  // extension with two panels gets two independently-dedupable tabs), so
  // re-clicking the same icon just re-focuses the existing tab. No wire fetch on
  // open/re-focus — the content is an `<iframe>` served straight off
  // `koma://extension/${extId}/index.html`, a SEPARATE origin from the host
  // chrome (the extension's own page manages its own state).
  | { id: string; kind: 'extension'; extId: string; panelId: string; title: string }
  // Coding panel file editor tab. `root` is the absolute workspace root;
  // `path` is relative to root. Stable id `coding:${root}:${path}`.
  | { id: string; kind: 'codingFile'; root: string; path: string; title: string; preview?: boolean }
  // Session attachment or other absolute-path preview (composer image chips).
  | { id: string; kind: 'localFile'; absPath: string; title: string }
  // One diagram canvas per `.koma/<name>.diag`. Stable id `diagram:${root}:${path}`,
  // so several diagrams stay open at once and a second click only focuses.
  | { id: string; kind: 'diagram'; root: string; path: string; title: string }
  // One design canvas per `.koma/<name>.kdsgn`. Stable id `design:${root}:${path}`.
  | { id: string; kind: 'design'; root: string; path: string; title: string }
  // Interactive terminal tab. `terminalId` is the PTY session id (host-minted);
  // content is an xterm.js instance reading from TerminalOutput push envelopes.
  | { id: string; kind: 'terminal'; terminalId: string; title: string }

