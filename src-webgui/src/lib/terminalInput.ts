import type { Terminal } from '@xterm/xterm'

/** True when the event target is xterm's hidden textarea or canvas wrapper. */
export function isTerminalInputTarget(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest('.xterm')
}

/**
 * WebKit/Wry often steal Backspace/Delete unless default is prevented on
 * keydown while xterm still handles the key. Linux PTYs usually expect DEL
 * (0x7f) for erase; normalize BS (0x08) when forwarding to the host.
 */
export function attachTerminalKeyGuards(term: Terminal) {
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== 'keydown') return true
    if (event.key === 'Backspace' || event.key === 'Delete') {
      event.preventDefault()
    }
    return true
  })
}

export function normalizeTerminalInput(data: string): string {
  return data.replace(/\x08/g, '\x7f')
}
