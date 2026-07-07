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
