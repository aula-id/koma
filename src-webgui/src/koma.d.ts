declare global {
  // JS -> Rust request payloads, tagged { t: 'req', ...GuiReq } by
  // useKoma().req() (see src/store/koma.ts).
  type GuiReq =
    | { r: 'Ready' }
    | { r: 'Submit'; text: string }
    | { r: 'SelectSession'; id: string }
    | { r: 'NewSession' }
    | { r: 'RefreshHub' }
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
