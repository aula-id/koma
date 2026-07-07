declare global {
  // JS -> Rust request payloads, tagged { t: 'req', ...GuiReq } by
  // useKoma().req() (see src/store/koma.ts).
  type GuiReq =
    | { r: 'Ready' }
    | { r: 'Submit'; text: string }
    | { r: 'SelectSession'; id: string }
    | { r: 'NewSession' }
    | { r: 'RefreshHub' }
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
    // mirrors RefreshHub/Submit's implicit-session pattern).
    | { r: 'RenameSession'; name: string }
    // MCP server CRUD — upserts by id/name (covers add + edit).
    | { r: 'SetMcpServer'; server: import('./types/config').McpServer }
    | { r: 'DeleteMcpServer'; id: string }
    | { r: 'EnableMcpServer'; id: string; enabled: boolean }
    // Provider CRUD — upserts by id (covers add + edit).
    | { r: 'SetProvider'; provider: import('./types/config').Provider }
    | { r: 'DeleteProvider'; id: string }
    // Model CRUD (roles + scope carried on the model itself) — upserts by id.
    | { r: 'SetModel'; model: import('./types/config').Model }
    | { r: 'DeleteModel'; id: string }
    // Live model-id catalogue fetch for a given provider; reply lands as the
    // ModelList push envelope.
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
