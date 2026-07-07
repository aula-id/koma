import type { Terminal } from '@xterm/xterm'

declare global {
  interface KomaBridge {
    term: Terminal
    write(b64: string): void
  }
  interface Window {
    __komaBg?: string
    __komaFg?: string
    __komaOS?: string
    __koma?: KomaBridge
    ipc?: { postMessage(msg: string): void }
  }
}

export {}
