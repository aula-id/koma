import type { MouseEvent } from 'react'

function post(msg: unknown) {
  try {
    window.ipc?.postMessage(JSON.stringify(msg))
  } catch {
    /* ipc unavailable */
  }
}

const DIRS = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'] as const

// Custom edge/corner resize handles — the window is undecorated, so these
// drive tao's drag_resize_window() instead of native OS chrome.
export function ResizeHandles() {
  function handleMouseDown(dir: (typeof DIRS)[number]) {
    return (e: MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0) return
      e.preventDefault()
      post({ t: 'winresize', dir })
    }
  }

  return (
    <>
      {DIRS.map((dir) => (
        <div key={dir} className={`rz rz-${dir}`} onMouseDown={handleMouseDown(dir)} />
      ))}
    </>
  )
}
