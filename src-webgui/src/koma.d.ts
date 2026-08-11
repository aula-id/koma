declare global {
  // JS -> Rust request payloads, tagged { t: 'req', ...GuiReq } by
  // useKoma().req() (see src/store/koma.ts).
  type GuiReq =
    | { r: 'Ready' }
    | { r: 'Submit'; text: string }
    | { r: 'SelectSession'; id: string }
    // `kill: true` disposes of the CURRENTLY-attached session (if any) by
    // killing its daemon before opening the native folder picker for the new
    // one — mirrors KillSession's kill semantics (daemon stops, session moves
    // to History). Omitted/false (the default, unchanged) keeps the current
    // session cooking in the background.
    | { r: 'NewSession'; kill?: boolean }
    | { r: 'RefreshHub' }
    // Kill a session — works for a background cooking session AND the
    // currently-attached one. The daemon shuts down but the session stays on
    // disk (moves to History) — contrast DeleteSession. If `id` is the
    // attached session, the host quits that daemon and transitions the
    // webview back to the hub/start state via its existing push flow; a
    // follow-up Hub push arrives automatically once the daemon is confirmed
    // dead.
    | { r: 'KillSession'; id: string }
    // Delete a HISTORY session from disk permanently (gone forever — unlike
    // KillSession, which just stops the daemon and keeps the session in
    // History). The host resolves the path and guards live sessions; a Hub
    // push follows.
    | { r: 'DeleteSession'; id: string }
    // Cancel an in-flight session switch (the loader's Cancel button). Best
    // effort: the attach can't be interrupted, so the host queues it and drops
    // to the swapper once the target lands (matches Rust GuiReq::CancelSwitch).
    | { r: 'CancelSwitch' }
    // Composer attach: raw bytes (clipboard-image paste / file-picker / drag
    // drop) the host persists to a scratch path and ingests via the existing
    // attachment core.
    | { r: 'AttachFile'; name: string; bytesB64: string; mime?: string }
    // Attach an existing workspace file (e.g. an omnisearch pick) by path —
    // no bytes need to cross the bridge.
    | { r: 'AttachPath'; path: string }
    // Omnisearch: fuzzy workspace file search (mirrors the @-palette).
    | { r: 'FileSearch'; query: string }
    // Drop a single staged attachment by its `[Image #N]` marker number.
    | { r: 'RemoveAttachment'; markerN: number }
    // Rename the foreground session (no id — daemon resolves current session,
    // mirrors RefreshHub/Submit's implicit-session pattern). Tag is `Rename`
    // to match the daemon's GuiReq variant.
    | { r: 'Rename'; name: string }
    // MCP server CRUD. Fields are FLAT (not a nested `server`) to match the
    // daemon's GuiReq. `uuid` is the daemon config uuid on edit, `null` for a
    // new server (the daemon mints one). `args`/`env` cross as the panel's
    // single-line STRING forms (space-joined args, "K=V, K2=V2" env).
    | {
        r: 'SetMcpServer'
        uuid: string | null
        name: string
        enabled: boolean
        transport: import('./types/config').Transport
        command: string
        args: string
        env: string
        url: string
      }
    | { r: 'DeleteMcpServer'; uuid: string }
    | { r: 'EnableMcpServer'; uuid: string; enabled: boolean }
    // MCP status refresh — request live per-server connection state.
    | { r: 'GetMcpStatus'; requestId: string }
    // Provider CRUD (flat). `uuid` is the daemon config uuid on edit, `null`
    // for a new provider.
    | { r: 'SetProvider'; uuid: string | null; name: string; endpoint: string; apiKey: string }
    | { r: 'DeleteProvider'; uuid: string }
    // Model CRUD (flat; roles + scope carried on the model). `uuid` is the
    // daemon config/override uuid on edit, `null` for a new model.
    // `providerUuid` is the serving provider's uuid; `route` is `null` when
    // unset. `scope` picks the global catalogue vs the session-local override.
    | {
        r: 'SetModel'
        uuid: string | null
        name: string
        modelId: string
        providerUuid: string
        route: string | null
        roles: import('./types/config').Role[]
        scope: import('./types/config').Scope
      }
    | { r: 'DeleteModel'; uuid: string; scope: import('./types/config').Scope }
    // Live model-id catalogue fetch for a given provider (by uuid); reply lands
    // as the ModelList push envelope.
    | { r: 'ListModels'; provider: string }
    // Live per-model ROUTE (OpenRouter upstream-endpoint) fetch: real provider
    // names + price/uptime for the chosen provider+model_id. Reply lands as the
    // RouteList push envelope. Non-OpenRouter providers reply with an empty
    // routes list (the UI then shows only the synthetic "Auto" row).
    | { r: 'ListRoutes'; provider: string; modelId: string }
    // Set the global agent mode (koma's Auto/Normal/Plan/Yolo). No id — the
    // daemon sets the process-global agent_mode via its set_agent_mode
    // choke-point (Plan enter/leave handling); the new token rides the next
    // Snapshot back to all clients. `mode` is the lowercase label token.
    | { r: 'SetMode'; mode: string }
    // Interrupt the running turn (the composer STOP button) — koma's
    // Esc-interrupt equivalent. No id: the daemon resolves the foreground
    // session (mirrors Submit's implicit-session pattern).
    | { r: 'Interrupt' }
    // `!<cmd>` composer shell shortcut: run `cmd` in the foreground session's
    // cwd, no model round-trip (koma's `!`-shell parity). Sent only while idle
    // (mirrors the TUI's busy guard); while working the composer falls through
    // to a normal `Submit` instead (queues as a steer).
    | { r: 'Shell'; cmd: string }
    // Ctrl+R composer parity: resend the last user turn (pop trailing
    // assistant messages + re-stream). Sent only when idle.
    | { r: 'Resend' }
    // Composer queued-steer-list clear button: cancel every pending mid-turn
    // steer at once (koma's Ctrl+X-with-pending-steers equivalent).
    | { r: 'CancelSteers' }
    // Rewind the conversation TO a user message by its index into
    // SessionSnapshot.messages (Conversation::messages()) — drops everything
    // after it, mirroring the TUI's double-Esc MessageRewind. No id: the daemon
    // resolves the foreground session. The daemon runs the existing
    // RewindToMessage action (abort in-flight turn + truncate live/sqlite +
    // refill the composer input); a non-user / out-of-range index is a clean
    // no-op host-side. The GUI mirrors the refill locally via refillComposer.
    | { r: 'RewindTo'; index: number }
    // Kill a single running subagent by its host-projected id (Explore panel
    // Agents row kill button). Numeric to match the daemon's `usize`.
    | { r: 'KillSubagent'; id: number }
    // Background a single running subagent by its host-projected id (Explore panel
    // Agents row background button) — flips it to detached without killing it, so it
    // keeps running and unblocks the main turn. Mirrors the TUI's Ctrl+B-on-selection.
    | { r: 'BackgroundSubagent'; id: number }
    // Background EVERY eligible running subagent at once (global Ctrl+B shortcut) —
    // mirrors the TUI composer's Ctrl+B.
    | { r: 'BackgroundAll' }
    // Kill a single running bg-bash job by its numeric id (Explore panel Bash
    // row kill button). The row id is `bash-<n>`; the numeric part matches the
    // daemon's `usize`.
    | { r: 'KillBash'; id: number }
    // Set the read-only STREAM VIEW: which sub-agent / bash job the Explore stream
    // tab is currently live-streaming (the ACTIVE stream tab, else both null = no
    // stream tab open). The host remembers it locally (the fold folds that target's
    // transcript / output tail into the push) AND forwards it to the daemon, which
    // un-suppresses the viewed detached sub-agent's live churn + projects the viewed
    // bash job's output tail. Exactly one is non-null in practice. Numeric ids to
    // match the daemon's `usize`. `session` PINS the ids to the current session (the
    // store's session.id) — sub-agent + bash ids are per-session counters, so the daemon
    // gates on it to avoid cross-session collisions (agent 0 exists in every session).
    | { r: 'SetStreamView'; subagent: number | null; bash: number | null; session: string | null }
    // Pin a chosen GLOBAL model as the session-local `main` override (the
    // quick-picker). `modelUuid` is the global model's uuid; `null` removes the
    // override and reverts to the Connector global main ("(inherit)").
    | { r: 'SetSessionMain'; modelUuid: string | null }
    // Set the active theme (palette) by its registry name (theme.rs PALETTES).
    // The host persists config.palette + live-repaints via the next Config push
    // (palette role set). Used by the onboarding theme picker.
    | { r: 'SetTheme'; name: string }
    // Onboarding "Koma Free" tile: mint (or reuse) the keyless free-tier
    // provider + model, mirroring the TUI first-run chooser's
    // `Action::SetupKomaFree`. Unit variant — no fields. The host re-pushes
    // Config once done, which flips useNeedsOnboarding and unmounts onboarding.
    | { r: 'SetupKomaFree' }
    // Answer a parked TOOL approval (risky/classifier-flagged pause). `approve`
    // = the TUI's y/Y (true) vs n/N/Esc (false). No id: the daemon resolves the
    // foreground session and applies Action::ApproveTool/DenyTool to the paused
    // `pending_tool_calls[tool_idx]`. Mirrors the daemon's ClientRequest::ApproveTool.
    | { r: 'ApproveTool'; approve: boolean }
    // Answer a parked PLAN decision (the `plan_ready` pause). `decision` is one
    // of "approve" (ApprovePlan) / "compact" (ApprovePlanCompact) / "deny"
    // (DenyPlan, "chat more"). No id: foreground session. Mirrors the daemon's
    // ClientRequest::PlanDecision.
    | { r: 'PlanDecision'; decision: 'approve' | 'compact' | 'deny' }
    // Trigger conversation compaction on demand (the UsageFooter's compact
    // button) — same effect as the TUI's /compact. No id: foreground session.
    | { r: 'Compact' }
    // Fetch the original/modified contents of a File-changed path for a Monaco
    // diff tab. `path` is exactly as the fileChanges record carries it. Reply
    // lands as the FileDiff push envelope (guaranteed for every request).
    | { r: 'FileDiff'; path: string }
    // Fetch the activity-bar Usage panel's LAST-7-DAYS preview: aggregate
    // totals, a 7-entry daily cost series, and the top 3 models by spend.
    // Reply lands as the UsagePreview push envelope (guaranteed for every
    // request, even detached — the host reads the global ledger directly).
    // Sent whenever the panel is (re)shown, and whenever the header's
    // all/session scope toggle flips. `scope` defaults to "all" (global) when
    // omitted; "session" filters to `sessionId`'s ledger rows only — required
    // for a "session" scope to mean anything, ignored otherwise.
    | { r: 'UsagePreview'; scope?: 'all' | 'session'; sessionId?: string }
    // Analytics tab: fetch a host-computed usage dashboard (KPI totals, time
    // series, per-model table, main-vs-sub role split) straight off the global
    // usage ledger. Host-owned; ALWAYS a reply (status ok/empty/error). Correlation
    // inputs (`reqSeq`/`scope`/`sessionId`/`range`/`metric`) are echoed so the
    // store can drop a stale reply. `range` is "today"/"7d"/"30d"/"year";
    // `metric` is "cost"/"tokens". A "session" scope with no sessionId is forced
    // to "all" host-side.
    | {
        r: 'Analytics'
        reqSeq: number
        scope?: 'all' | 'session'
        sessionId?: string
        range?: 'today' | '7d' | '30d' | 'year'
        metric?: 'cost' | 'tokens'
      }
    // Fetch the Settings tab's Session-section values (name / workdir / toggles /
    // internet mode) + the active palette. Sent when the tab opens or re-activates.
    // Reply lands as the SettingsValues push envelope (guaranteed for every request,
    // even detached — the host answers from global config with defaults).
    | { r: 'GetSettings' }
    // Commit a PARTIAL settings update from the Settings tab's Session section. Only
    // the present fields are sent; the host applies each through the same per-field
    // logic the TUI settings save uses, persists, and re-pushes SettingsValues. Name
    // changes go via `Rename`, palette changes via `SetTheme` (not here).
    | {
        r: 'SetPrefs'
        shortSend?: boolean
        slidingCache?: boolean
        bashSaving?: boolean
        codingAutosave?: boolean
        internetMode?: string
        workdir?: string[]
      }
    // Composer EFFORT pill opened: fetch the derived `/effort` menu (TUI
    // parity) for the foreground session's current model. Attached-only (like
    // Interrupt/SetPrefs — no session ⇒ no-op, the picker just stays in its
    // loading state). Reply lands as the EffortOptions push envelope
    // (guaranteed for every request once attached: loading/unsupported/ready).
    | { r: 'GetEffortOptions' }
    // EFFORT picker row pick: persist the chosen effort level ("default" =
    // model default). Attached-only, like SetPrefs. Reply lands as a fresh
    // SettingsValues push (the picker's trigger-pill label updates off that
    // same channel — no dedicated ack).
    | { r: 'SetEffort'; effort: string }
    // Agents dashboard: fetch the current agent list + model/provider
    // catalogues. Reply lands as the AgentsValues push envelope — ALWAYS,
    // even un-attached (the host answers with built-in + global agents only,
    // straight off global config). Sent once when the Agents sidebar panel
    // mounts; every SetAgent/DeleteAgent reply also re-pushes AgentsValues,
    // so no polling or re-request is needed after a mutation.
    | { r: 'GetAgents' }
    // Agent create/edit. `originalName` null = CREATE (uses `scope` verbatim);
    // non-null = EDIT, keyed by the agent's pre-edit name (equal to `name` for
    // a non-rename edit, different for a rename) — the daemon derives the
    // actual write tier from the target agent's own current source on an edit
    // (a builtin edit auto-becomes a session override) and only falls back to
    // `scope` if that named agent no longer exists, so `scope` must still be
    // sent (required field) but is otherwise disregarded on edit. Matches the
    // daemon's GuiReq::SetAgent (mirrors AgentDef's own field set).
    | {
        r: 'SetAgent'
        originalName: string | null
        scope: 'global' | 'session'
        name: string
        description: string
        conditions: string
        modelUuid: string | null
        tools: string[]
        prompt: string
        // Client-side monotonically-increasing seq for stale-reply protection;
        // echoed in the confirmatory AgentsValues/AgentOp push.
        reqSeq?: number
      }
    // Delete an agent. Unlike SetAgent's edit path, `scope` here is used
    // VERBATIM as the tier to delete from — "session" needs a live session
    // dir (errors otherwise), anything else deletes from global. Builtins are
    // delete-rejected daemon-side. Reply lands as a fresh AgentsValues push.
    | { r: 'DeleteAgent'; scope: 'global' | 'session'; name: string; reqSeq?: number }
    // OAuth login screen: fetch the current connections + available login
    // providers. Dual-routed like GetSettings/GetAgents — safe to send with NO
    // session attached (the host answers from ~/.koma/config.json + the
    // provider registry). Reply lands as the OAuthState push envelope, always.
    | { r: 'GetOAuthState' }
    // Start a login flow. `provider` is the wire id of one of the CURRENT
    // `OAuthState.providers` entries (data-driven — today "codex" (PKCE
    // browser), "kilocode" (device code), "xai" (device code), "claudeai"
    // (PKCE browser), "komarun" (PKCE browser), or "codex_paste" (manual
    // token), but never hardcode this list client-side). Dual-routed like
    // GetOAuthState/DeleteOAuthConn — works with NO session attached (the host
    // runs the flow itself); the attached daemon path is unchanged. Progress
    // streams back as further OAuthState pushes (phase transitions).
    | { r: 'StartOAuth'; provider: string }
    // Complete the "paste token" flow (phase 'paste') with a raw access
    // `token`. Attached-only — the paste screen only ever follows an
    // in-session `StartOAuth('codex_paste')`. An empty/whitespace token is
    // rejected daemon-side (re-surfaces phase 'paste') — never crashes, just
    // re-prompts.
    | { r: 'SubmitOAuthPaste'; token: string }
    // Cancel an in-flight login flow, back to phase 'idle'. Dual-routed like
    // StartOAuth — works with NO session attached (aborts the host-local flow,
    // a no-op if none is in flight).
    | { r: 'CancelOAuth' }
    // Delete a persisted OAuth connection by uuid. Dual-routed like
    // GetOAuthState/DeleteAgent — works with NO session attached. Reply lands
    // as a fresh OAuthState push (phase 'idle', conns updated).
    | { r: 'DeleteOAuthConn'; uuid: string }
    // Open `url` in the SYSTEM browser (never inside the webview) — e.g. the
    // Settings "Account" section's "Manage account on koma.run" link. Pure
    // host-local side effect (spawns the OS opener, fire-and-forget): no
    // session/attach needed, no reply, no push.
    | { r: 'OpenExternal'; url: string }
    // Extension STORE (koma.run marketplace). Browse/detail hit the PUBLIC store
    // endpoints; install/uninstall mutate the live daemon (managers + config), so
    // the whole family is forwarded to the attached daemon. Replies land as the
    // StoreCatalogue / StoreItemDetail / InstalledExtensions / ExtensionOpResult
    // push envelopes. Attached-only (a GUI window always has a session daemon
    // attached in normal use).
    // Browse the catalogue with optional full-text (`query`) + `category`
    // filters. Reply lands as StoreCatalogue.
    | { r: 'StoreBrowse'; query?: string; category?: string }
    // Fetch one extension's full detail (long description + contributes/requires
    // + versions). Reply lands as StoreItemDetail.
    | { r: 'StoreDetail'; id: string }
    // Install `id` (optional `version`, else latest): download → verify signature
    // → unpack → register → spawn. Reply lands as ExtensionOpResult then a fresh
    // InstalledExtensions. Requires a signed-in koma.run account (else the
    // ExtensionOpResult carries "sign in to koma.run to install").
    | { r: 'InstallExtension'; id: string; version?: string }
    // Uninstall `id`: purge contributions + stop + remove dir + registry entry.
    // Reply lands as ExtensionOpResult then a fresh InstalledExtensions.
    | { r: 'UninstallExtension'; id: string }
    // Fetch the locally-installed extension registry. Reply lands as
    // InstalledExtensions.
    | { r: 'ListInstalledExtensions' }
    // Fetch full detail of one locally-installed extension: registry fields +
    // on-disk manifest contributions (tools/models/panels/sub-agents). Reply
    // lands as InstalledExtensionDetail.
    | { r: 'GetInstalledExtensionDetail'; id: string }
    // A GUI extension PANEL's request to its backing extension daemon (W9
    // panel bridge). The panel iframe (`koma://extension/<extId>/…`) posts a
    // `{koma:'panel', v:1, kind:'msg', reqId, payload}` message; the
    // panelBridge listener (lib/panelBridge.ts) attributes it via the
    // registry (never trusts `extId`/`panelId` from message content) and
    // forwards it here. The host relays it to the attached daemon as
    // ClientRequest::ExtPanelMsg, which auto-starts the extension + invokes
    // its `panel.msg` and answers OUT-OF-BAND with an ExtPanelReply push the
    // host re-pushes (matches the Rust GuiReq::ExtPanelMsg). `reqId`
    // correlates the reply; `payload` is extension-defined. Attached-only —
    // with no attached daemon the panelBridge listener itself replies
    // locally instead of sending this.
    | { r: 'ExtPanelMsg'; extId: string; panelId: string; reqId?: string; payload: unknown }
    // Source Control "GIT" panel opened / refreshed: fetch a host-computed git
    // status (branch, ahead/behind, staged + unstaged file lists) for the
    // foreground session's repo. Serviced ENTIRELY host-side — works
    // regardless of attach state. Reply lands as the GitStatus push envelope
    // (matches the Rust GuiReq::GitStatus unit variant).
    | { r: 'GitStatus' }
    // The GIT panel's file row clicked: fetch a host-computed git diff for
    // `path` — `staged` selects index-vs-HEAD (true, the STAGED changes) or
    // worktree-vs-index (false, the UNSTAGED changes) — to open a Monaco diff
    // tab. Reply lands as the GitDiff push envelope.
    | { r: 'GitDiff'; path: string; staged: boolean }
    // GIT panel "Stage All" header action / a row's hover "+" button: `git add --`
    // every path in `paths` (repo-root-relative, straight off a GitFileEntry). Reply
    // lands as a one-shot GitOp push, immediately followed by a fresh GitStatus push.
    | { r: 'GitStage'; paths: string[] }
    // GIT panel "Unstage All" header action / a staged row's hover "−" button:
    // `git restore --staged --` every path in `paths`. Same reply pattern as GitStage.
    | { r: 'GitUnstage'; paths: string[] }
    // GIT panel "Discard All Changes" header action / an unstaged row's discard
    // button — destructive, so the UI gates this behind an inline confirm before ever
    // sending it. Untracked paths are deleted from disk; tracked paths are reset from
    // the index (staged content is never touched). Same reply pattern as GitStage.
    | { r: 'GitDiscard'; paths: string[] }
    // GIT panel commit box submit: `git commit -m <message>` whatever is currently
    // staged. An empty/whitespace message is rejected host-side (GitOp.error set, no
    // git invocation). Reply lands as GitOp then a fresh GitStatus; the commit box
    // clears its draft on a successful (ok:true) reply.
    | { r: 'GitCommit'; message: string }
    // GIT panel key-picker changed: assign the foreground session's repo to use
    // SSH key `name` (a vault key) for remote ops, or clear the assignment
    // (`name: null` — "Default (system ssh)"). No dedicated reply; a fresh
    // GitStatus push reflects the new `keyName`.
    | { r: 'SetGitKey'; name: string | null }
    // GIT panel Fetch button: `git fetch --prune` using the repo's assigned key's
    // SSH override if one is set. Reply lands as a one-shot GitOp push (op:
    // 'fetch'), immediately followed by a fresh GitStatus push.
    | { r: 'GitFetch' }
    // GIT panel Pull button: `git pull --ff-only` (fails loudly on divergence
    // rather than merging/leaving a half-merged tree). Same reply pattern as
    // GitFetch.
    | { r: 'GitPull' }
    // GIT panel Push button. Same reply pattern as GitFetch.
    | { r: 'GitPush'; mode?: 'automatic' | 'plain' | 'set-upstream' | 'force-with-lease'; root?: string | null }
    // Commit-graph tab: fetch a host-computed paginated commit graph across
    // every ref (GitKraken-style). `limit`/`skip` page it (200 per page); reply
    // lands as the GitGraph push envelope — ALWAYS a reply, serviced entirely
    // host-side (works regardless of attach state). Matches the Rust
    // GuiReq::GitGraph { limit, skip }.
    | { r: 'GitGraph'; limit: number; skip: number }
    // Commit-graph row click: fetch one commit's full metadata (incl. body) +
    // first-parent changed-file list. Reply lands as the CommitDetail push.
    | { r: 'GitCommitDetail'; sha: string }
    // Commit-detail file-row click: fetch one file's diff at `sha` vs its first
    // parent, for a Monaco diff tab. Reply lands as the CommitDiff push.
    | { r: 'GitCommitDiff'; sha: string; path: string }
    // Settings "SSH Keys" section opened / refreshed: fetch the host key vault's
    // current key list (`<~/.koma>/keys/`). GUI-only, manual, user-owned vault —
    // completely separate from the model's own git credential machinery. Serviced
    // ENTIRELY host-side — works regardless of attach state. Reply lands as the
    // KeyList push envelope.
    | { r: 'KeyList' }
    // Generate a fresh passphrase-less ed25519 keypair named `name` (`comment`
    // defaults to "koma" when blank). Reply lands as a one-shot KeyOp push,
    // immediately followed by a fresh KeyList push.
    | { r: 'KeyGenerate'; name: string; comment: string }
    // Import an EXISTING private key (`name` + pasted `privateKey` text) into the
    // vault; the host derives + writes the matching public half. Same reply
    // pattern as KeyGenerate.
    | { r: 'KeyImport'; name: string; privateKey: string }
    // Reveal key `name`'s contents — `private: false` for "Copy public key" (no
    // confirmation needed), `private: true` for "Reveal private key" (gated
    // behind a deliberate click + warning UI-side). Reply lands as a one-shot
    // KeyReveal push.
    | { r: 'KeyReveal'; name: string; private: boolean }
    // Delete keypair `name` (both halves, best-effort). Same reply pattern as
    // KeyGenerate.
    | { r: 'KeyDelete'; name: string }
    // Branch-switcher popover (footer/GitPanel) or graph context menu opened
    // (G4): fetch every local + remote-tracking branch. Serviced ENTIRELY
    // host-side — works regardless of attach state. Reply lands as the
    // BranchList push envelope.
    | { r: 'GitBranchList'; requestId?: number | null }
    | { r: 'GitRepos' }
    | { r: 'SetActiveRepo'; root: string }
    // Branch-switcher pick / graph "Checkout"/"Checkout commit" (G4 — SAFE
    // only, never `--force`): switch (or detach onto) `ref` — a branch name or
    // a sha. Reply lands as a one-shot GitOp push (`op: 'checkout'`),
    // immediately followed by a fresh GitStatus push.
    | { r: 'GitCheckout'; ref: string; root?: string | null }
    // Branch-switcher "+ Create new branch" / graph "Create branch here…"
    // (G4 — SAFE only): create branch `name` from `start` (`null` = current
    // HEAD), optionally switching to it immediately (`checkout`). Reply lands
    // as a one-shot GitOp push (`op: 'createBranch'`), immediately followed by
    // a fresh GitStatus push.
    | { r: 'GitCreateBranch'; name: string; start: string | null; checkout: boolean; root?: string | null }
    // Commit-graph row context menu "Cherry-pick" (G5c — may conflict; the
    // follow-up GitStatus push's inProgress/conflicted fields carry that
    // state, not this request's reply alone). Reply lands as a one-shot GitOp
    // push (`op: 'cherryPick'`), immediately followed by a fresh GitStatus push.
    | { r: 'GitCherryPick'; sha: string }
    // Commit-graph row context menu "Revert" (G5c). Same conflict reasoning
    // as GitCherryPick. Reply lands as a one-shot GitOp push (`op: 'revert'`).
    | { r: 'GitRevert'; sha: string }
    // Commit-graph row context menu "Reset branch to here" (G5c). `mode` is
    // 'soft'/'mixed'/'hard' — 'hard' DISCARDS uncommitted changes; the UI
    // gates this behind a strong inline confirm BEFORE sending it. Reply
    // lands as a one-shot GitOp push (`op: 'reset'`).
    | { r: 'GitReset'; sha: string; mode: 'soft' | 'mixed' | 'hard' }
    // Branch-switcher / graph context menu "Merge into current branch" (G5c —
    // may conflict, same reasoning as GitCherryPick). Reply lands as a
    // one-shot GitOp push (`op: 'merge'`).
    | { r: 'GitMerge'; ref: string }
    // Rebase onto `upstream` (G5c/G6). `branch` is a branch name for the
    // GitKraken-style drag-to-rebase (drag a branch chip onto a commit/ref —
    // that branch is checked out + rebased onto `upstream`, current branch
    // untouched), or `null` for the plain "rebase current branch" op. May
    // conflict. Reply lands as a one-shot GitOp push (`op: 'rebase'`).
    | { r: 'GitRebase'; upstream: string; branch: string | null }
    // The conflict banner's Abort button (G5c). `kind` is
    // 'merge'/'rebase'/'cherry-pick'/'revert' (echoing GitStatus.inProgress).
    // Reply lands as a one-shot GitOp push (`op: 'abort'`), immediately
    // followed by a fresh GitStatus push.
    | { r: 'GitOpAbort'; kind: string }
    // The conflict banner's Continue button (G5c). Same `kind` values as
    // GitOpAbort. Reply lands as a one-shot GitOp push (`op: 'continue'`) —
    // git refuses (and the GitOp reply carries an error) if conflicts remain.
    | { r: 'GitOpContinue'; kind: string }
    // Toolbar "Stash" button (GK4c): `git stash push` (tracked + staged
    // changes only, matching plain `git stash`'s own default). Reply lands as
    // a one-shot GitOp push (`op: 'stash'`) — the host already follows every
    // GitOp mutation with its own fresh GitStatus push, so the toolbar's
    // staged/unstaged counts update on their own; the GitOp reducer also
    // re-fetches the stash list.
    | { r: 'GitStash' }
    // Toolbar "Pop" button (GK4c): `git stash pop`. May conflict — same
    // reasoning as GitCherryPick (the existing G5 conflict banner surfaces
    // it via the host's follow-up GitStatus push). Reply lands as a one-shot
    // GitOp push (`op: 'stashPop'`).
    | { r: 'GitStashPop' }
    // Toolbar mount / stash-op follow-up (GK4c): fetch every `git stash list`
    // entry for the Stash/Pop buttons' counts. Serviced entirely host-side.
    // Reply lands as the StashList push envelope.
    | { r: 'GitStashList' }
    // Bubble/activity chart (GK5b): fetch per-commit author/date/lines-changed
    // rows for the ACTIVE branch, optionally narrowed to one pathspec. `path`
    // null means the whole branch. Reply lands as the Activity push envelope
    // (matches the Rust GuiReq::GitActivity { path, limit }).
    | { r: 'GitActivity'; path: string | null; limit: number }
    // ─── Coding panel: workspace file operations ──────────────────────────
    // List a directory's immediate children. `root` is one of the session's
    // configured workspace roots (absolute path); `path` is relative to root
    // (empty string = root itself). Reply lands as FileTree push.
    | { r: 'FileTree'; root: string; path: string; requestId: string }
    // Read a text file's content. Reply lands as FileRead push.
    | { r: 'FileRead'; root: string; path: string; requestId: string }
    // Save a text file. `expectedFingerprint` must match the disk state from
    // the most recent FileRead; mismatch = conflict (stale save rejected).
    // Reply lands as FileSave push.
    | { r: 'FileSave'; root: string; path: string; content: string; expectedFingerprint: string; requestId: string }
    // Create a new file or directory. `kind` is "file" or "dir". Reply lands
    // as FileCreate push.
    | { r: 'FileCreate'; root: string; path: string; kind: 'file' | 'dir'; requestId: string }
    // Rename/move a file or directory. Both paths are relative to `root`.
    // v1: within the same root only. Reply lands as FileRename push.
    | { r: 'FileRename'; root: string; oldPath: string; newPath: string; requestId: string }
    // Delete a file or directory (recursive for dirs). Reply lands as FileDelete push.
    | { r: 'FileDelete'; root: string; path: string; requestId: string }
    // Write an error message to the global error log (`~/.koma/error.log`). Used by
    // the React error boundary to log runtime errors that only occur in the built
    // app (not in dev mode). No reply, no session needed.
    | { r: 'WriteErrorLog'; message: string }

  // ─── Linker daemon import graph ─────────────────────────────────────
  // Fetch the linker daemon's code-dependency graph. `path` focuses on
  // one file (null = overview mode). `depth` limits traversal depth.
  // `direction` picks which edges to include. `filterRoots` and
  // `filterLanguages` narrow the view server-side. Reply lands as the
  // ImportGraph push envelope.
  | {
      r: 'ImportGraph'
      path?: string | null
      depth?: number | null
      direction?: 'dependencies' | 'dependents' | 'both' | null
      filterRoots?: string[] | null
      filterLanguages?: string[] | null
    }
  // Impact analysis: transitive reverse deps for a file.
  | {
      r: 'ImportGraphImpact'
      path: string
      depth?: number | null
      requestId: string
    }

  // ─── Coding push envelope shapes (Rust → JS) ────────────────────────────
  // Every reply echoes the relevant root/path/requestId for stale-reply rejection.
  type FileTreeEntry = {
    name: string
    path: string       // relative to root
    isDir: boolean
  }

  type CodingPush =
    | { k: 'FileTree'; root: string; path: string; requestId: string; entries: FileTreeEntry[]; error: string | null }
    | { k: 'FileRead'; root: string; path: string; requestId: string; content: string | null; fingerprint: string; binary: boolean; tooLarge: boolean; error: string | null }
    | { k: 'FileSave'; root: string; path: string; requestId: string; fingerprint: string; error: string | null }
    | { k: 'FileCreate'; root: string; path: string; requestId: string; error: string | null }
    | { k: 'FileRename'; root: string; oldPath: string; newPath: string; requestId: string; error: string | null }
    | { k: 'FileDelete'; root: string; path: string; requestId: string; error: string | null }

  interface KomaClient {
    // Rust -> JS: host calls this via evaluate_script with a JSON-encoded
    // push envelope; forwarded straight into the koma store's reducer.
    push(json: string): void
  }

  interface Window {
    __komaOS?: string
    __komaClient?: KomaClient
    ipc?: { postMessage(msg: string): void }
    komaIpc?: (req: GuiReq) => void
  }
}

export {}
