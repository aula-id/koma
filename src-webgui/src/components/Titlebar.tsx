import type { MouseEvent } from 'react'

export type Platform = 'macos' | 'linux' | 'windows'

// window.__komaOS is injected by the Rust host (run_gui) before this script
// runs. Falls back to 'linux' (title-left / controls-right layout).
export function getPlatform(): Platform {
  const os = window.__komaOS
  return os === 'macos' || os === 'windows' ? os : 'linux'
}

function post(msg: unknown) {
  try {
    window.ipc?.postMessage(JSON.stringify(msg))
  } catch {
    /* ipc unavailable */
  }
}

// Custom titlebar: the window is undecorated (tao `with_decorations(false)`),
// so drag / minimize / maximize / close all have to be driven host-side via
// ipc — the host's `event_loop.run` closure calls the actual tao `Window`
// methods (drag_window / set_minimized / set_maximized / exit).
export function Titlebar() {
  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (e.button !== 0) return
    const target = e.target as HTMLElement
    if (target.closest('.win-btn')) return // buttons handle themselves
    if (e.detail === 2) {
      post({ t: 'win', a: 'max' }) // dbl-click = toggle max
      return
    }
    post({ t: 'win', a: 'drag' })
  }

  return (
    <div id="titlebar" onMouseDown={handleMouseDown}>
      <span id="title">koma</span>
      <div id="winctl">
        <button
          className="win-btn"
          id="btn-min"
          aria-label="Minimize"
          onClick={() => post({ t: 'win', a: 'min' })}
        >
          &#x2013;
        </button>
        <button
          className="win-btn"
          id="btn-max"
          aria-label="Maximize"
          onClick={() => post({ t: 'win', a: 'max' })}
        >
          &#x2610;
        </button>
        <button
          className="win-btn win-close"
          id="btn-close"
          aria-label="Close"
          onClick={() => post({ t: 'win', a: 'close' })}
        >
          &#x2715;
        </button>
      </div>
    </div>
  )
}
