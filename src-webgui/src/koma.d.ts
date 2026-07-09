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
      }
    // Delete an agent. Unlike SetAgent's edit path, `scope` here is used
    // VERBATIM as the tier to delete from — "session" needs a live session
    // dir (errors otherwise), anything else deletes from global. Builtins are
    // delete-rejected daemon-side. Reply lands as a fresh AgentsValues push.
    | { r: 'DeleteAgent'; scope: 'global' | 'session'; name: string }

  interface KomaClient {
    // Rust -> JS: host calls this via evaluate_script with a JSON-encoded
    // push envelope; forwarded straight into the koma store's reducer.
    push(json: string): void
  }

  interface Window {
    __komaOS?: string
    __komaClient?: KomaClient
    ipc?: { postMessage(msg: string): void }
  }
}

export {}
