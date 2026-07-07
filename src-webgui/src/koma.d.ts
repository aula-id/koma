import type { Terminal } from '@xterm/xterm'

declare global {
  interface KomaBridge {
    term: Terminal
    write(b64: string): void
  }

  // JS -> Rust request payloads, tagged { t: 'req', ...GuiReq } by
  // useKoma().req() (see src/store/koma.ts).
  type GuiReq =
    | { r: 'Ready' }
    | { r: 'Submit'; text: string }
    | { r: 'SelectSession'; id: string }
    | { r: 'NewSession' }

  interface KomaClient {
    // Rust -> JS: host calls this via evaluate_script with a JSON-encoded
    // push envelope; forwarded straight into the koma store's reducer.
    push(json: string): void
  }

  interface Window {
    __komaBg?: string
    __komaFg?: string
    __komaOS?: string
    __koma?: KomaBridge
    __komaClient?: KomaClient
    ipc?: { postMessage(msg: string): void }
  }
}

export {}
