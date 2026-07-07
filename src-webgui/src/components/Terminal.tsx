import '@xterm/xterm/css/xterm.css'

import { useEffect, useRef } from 'react'
import { Terminal as XTerm } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { Unicode11Addon } from '@xterm/addon-unicode11'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { ClipboardAddon } from '@xterm/addon-clipboard'
import { WebglAddon } from '@xterm/addon-webgl'

const HEX_RE = /^#[0-9a-fA-F]{6}$/

export function Terminal() {
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // The host (Rust, run_gui) resolves koma's CONFIGURED palette canvas bg
    // (view::theme::palette(cfg).bg) and injects it as window.__komaBg via a
    // wry initialization script, so it's set before this script runs. Falls
    // back to black if missing/malformed (matches the palette's own default).
    let komaBg = window.__komaBg && HEX_RE.test(window.__komaBg) ? window.__komaBg : '#000000'
    // Same deal for the FOREGROUND: the host resolves koma's CONFIGURED
    // palette fg (view::theme::palette(cfg).fg) and injects it as
    // window.__komaFg. Falls back to the old hardcoded color if missing/malformed.
    let komaFg = window.__komaFg && HEX_RE.test(window.__komaFg) ? window.__komaFg : '#c8d3f5'

    document.documentElement.style.setProperty('--koma-bg', komaBg)
    document.documentElement.style.setProperty('--koma-fg', komaFg)

    const term = new XTerm({
      fontFamily: '"KomaMono", monospace',
      fontSize: 14,
      cursorBlink: true,
      allowProposedApi: true,
      theme: { background: komaBg, foreground: komaFg },
      scrollback: 10000,
    })

    const fit = new FitAddon()
    term.loadAddon(fit)
    try {
      term.loadAddon(new Unicode11Addon())
      term.unicode.activeVersion = '11'
    } catch {
      /* addon unavailable */
    }
    try {
      term.loadAddon(new WebLinksAddon())
    } catch {
      /* addon unavailable */
    }
    try {
      term.loadAddon(new ClipboardAddon())
    } catch {
      /* addon unavailable */
    }

    term.open(containerRef.current!)

    // Live palette sync: koma (in GUI mode) emits its canvas bg + titlebar fg
    // via a private OSC 5380 whenever the palette changes, as
    // `#rrggbb,#rrggbb` (bg first, fg second); repaint the xterm theme + CSS
    // vars to match.
    try {
      term.parser.registerOscHandler(5380, (data) => {
        const parts = String(data).split(',')
        const bg = parts[0]
        const fg = parts[1]
        if (bg && HEX_RE.test(bg)) {
          komaBg = bg
          term.options.theme = { ...term.options.theme, background: bg }
          document.documentElement.style.setProperty('--koma-bg', bg)
        }
        if (fg && HEX_RE.test(fg)) {
          komaFg = fg
          term.options.theme = { ...term.options.theme, foreground: fg }
          document.documentElement.style.setProperty('--koma-fg', fg)
        }
        return true // handled — do not render the sequence
      })
    } catch {
      /* parser API unavailable — static window.__komaBg/__komaFg still applies */
    }

    // Ctrl+Shift+C copies the current selection, Ctrl+Shift+V pastes from the
    // system clipboard. Plain Ctrl+C is left alone so it still sends SIGINT
    // to the koma client running in the pty. The clipboard addon handles OSC
    // 52 from the app side; this handles user-initiated copy/paste from the UI.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === 'keydown' && e.ctrlKey && e.shiftKey) {
        const k = e.key.toLowerCase()
        if (k === 'c') {
          const sel = term.getSelection()
          if (sel && navigator.clipboard) {
            navigator.clipboard.writeText(sel).catch(() => {})
          }
          return false
        }
        if (k === 'v') {
          if (navigator.clipboard) {
            navigator.clipboard
              .readText()
              .then((t) => {
                if (t) {
                  const b = new TextEncoder().encode(t)
                  let s = ''
                  for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i])
                  post({ t: 'data', d: btoa(s) })
                }
              })
              .catch(() => {})
          }
          return false
        }
      }
      return true
    })

    try {
      const _webgl = new WebglAddon()
      _webgl.onContextLoss(() => {
        try {
          _webgl.dispose()
        } catch {
          /* already disposed */
        }
      })
      term.loadAddon(_webgl)
    } catch {
      /* WebGL unavailable — xterm falls back to its DOM renderer */
    }

    window.__koma = {
      term,
      // pty -> xterm: host base64's raw pty bytes; decode to a Uint8Array so
      // multibyte UTF-8 (braille, box-drawing) survives intact.
      write(b64: string) {
        const bin = atob(b64)
        const arr = new Uint8Array(bin.length)
        for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
        term.write(arr)
      },
    }

    function post(obj: unknown) {
      try {
        window.ipc?.postMessage(JSON.stringify(obj))
      } catch {
        /* ipc unavailable */
      }
    }

    // keystrokes / paste -> pty (UTF-8 bytes, base64'd)
    term.onData((data) => {
      const bytes = new TextEncoder().encode(data)
      let bin = ''
      for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i])
      post({ t: 'data', d: btoa(bin) })
    })

    // xterm computed a new grid size -> pty (TIOCSWINSZ -> SIGWINCH)
    term.onResize(({ cols, rows }) => {
      post({ t: 'resize', cols, rows })
    })

    // window resize -> refit (fit() triggers onResize above)
    const onWindowResize = () => {
      try {
        fit.fit()
      } catch {
        /* ignore */
      }
    }
    window.addEventListener('resize', onWindowResize)

    // ResizeObserver on the container catches container size changes the
    // window 'resize' event misses (e.g. layout/DPI shifts without a window
    // resize), so any non-remainder margin gets refit away too.
    // rAF-debounced so rapid observer callbacks collapse to one fit() per frame.
    let ro: ResizeObserver | undefined
    if (window.ResizeObserver) {
      let _pending = false
      ro = new ResizeObserver(() => {
        if (_pending) return
        _pending = true
        requestAnimationFrame(() => {
          _pending = false
          try {
            fit.fit()
          } catch {
            /* ignore */
          }
        })
      })
      ro.observe(containerRef.current!)
    }

    // Gate the initial fit/ready handshake on the bundled font actually being
    // loaded: xterm measures cell size from the active font, so if we fit()
    // while the fallback font is still active and KomaMono swaps in after,
    // the grid ends up mis-sized. Fire `ready` exactly once, only after this.
    function boot() {
      try {
        fit.fit()
      } catch {
        /* ignore */
      }
      post({ t: 'ready' })
    }
    if (document.fonts?.ready) {
      Promise.resolve(document.fonts.load('14px "KomaMono"'))
        .catch(() => {})
        .then(() => document.fonts.ready)
        .then(boot)
        .catch(boot)
    } else {
      boot()
    }

    return () => {
      window.removeEventListener('resize', onWindowResize)
      ro?.disconnect()
      term.dispose()
      window.__koma = undefined
    }
  }, [])

  return (
    <div
      ref={containerRef}
      className="absolute inset-0"
      style={{ backgroundColor: 'var(--koma-bg, #000)' }}
    />
  )
}
