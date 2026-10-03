import type { Terminal } from '@xterm/xterm'

/** True when the event target is xterm's hidden textarea or canvas wrapper. */
export function isTerminalInputTarget(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest('.xterm')
}

export function isMacTerminalPlatform(): boolean {
  if (typeof navigator === 'undefined') return false
  const ua = navigator.userAgent || ''
  const platform = navigator.platform || ''
  return /Mac|iPhone|iPad|iPod/.test(platform) || (/\bMac OS X\b/.test(ua) && !/\bWindows\b/.test(ua))
}

/**
 * WebKit/Wry often steal Backspace/Delete unless default is prevented on
 * keydown while xterm still handles the key. On macOS, xterm + WKWebView can
 * emit the wrong bytes for zsh (space / "~"); inject canonical xterm sequences
 * and skip xterm's broken handler. Linux PTYs usually expect DEL (0x7f) for
 * erase; normalize BS (0x08) when forwarding to the host.
 */
export function attachTerminalKeyGuards(term: Terminal) {
  const mac = isMacTerminalPlatform()

  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== 'keydown') return true
    if (event.key !== 'Backspace' && event.key !== 'Delete') return true

    event.preventDefault()

    if (mac) {
      term.input(event.key === 'Backspace' ? '\x7f' : '\x1b[3~', true)
      return false
    }

    return true
  })
}

export function normalizeTerminalInput(data: string): string {
  return data.replace(/\x08/g, '\x7f')
}
