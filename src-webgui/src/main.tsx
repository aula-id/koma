import React from 'react'
import ReactDOM from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import './styles.css'

// Monaco cancels in-flight async work (completion/hover/definition, model
// dispose on tab close / session switch) by rejecting with CancellationError
// whose message is the single-l spelling "Canceled". Nothing in the app awaits
// those promises, so WebKit reports Unhandled Promise Rejection on every
// coding teardown. Swallow only that known cancellation shape.
function isMonacoCanceled(reason: unknown): boolean {
  if (reason == null || typeof reason !== 'object') return false
  const r = reason as { name?: unknown; message?: unknown; code?: unknown }
  if (r.name === 'Canceled' || r.name === 'CancellationError') return true
  if (r.message === 'Canceled' || r.message === 'Cancelled') return true
  // vscode-jsonrpc / monaco sometimes use numeric Cancellation code 1
  if (r.code === 1 && (r.name === 'Canceled' || r.message === 'Canceled')) return true
  return false
}

if (typeof window !== 'undefined') {
  window.addEventListener('unhandledrejection', (ev) => {
    if (isMonacoCanceled(ev.reason)) {
      ev.preventDefault()
    }
  })
}

// First paint must not wait on webfonts — the static #koma-boot-splash in
// index.html is already on screen. React replaces #root as soon as the
// module graph is ready. Fonts keep loading in the background.
function renderApp() {
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <RouterProvider router={router} />
    </React.StrictMode>
  )
}

// Chromium ignores autocomplete="off" on <body>. Force it on every
// input/textarea so browser autofill suggestions don't appear.
if (typeof MutationObserver !== 'undefined') {
  const disableAutocomplete = (nodes: Node[]) => {
    for (const n of nodes) {
      if (n instanceof HTMLElement && (n.tagName === 'INPUT' || n.tagName === 'TEXTAREA')) {
        n.setAttribute('autocomplete', 'off')
      }
    }
  }
  const observer = new MutationObserver((mutations) => {
    for (const m of mutations) disableAutocomplete(Array.from(m.addedNodes))
  })
  observer.observe(document.documentElement, { childList: true, subtree: true })
}

if (typeof document !== 'undefined' && document.fonts) {
  const fonts = ['400', '500', '700'].map((w) => document.fonts.load(`${w} 12px KomaMono`))
  void Promise.all(fonts)
    .then(() => document.fonts.ready)
    .catch(() => undefined)
}

renderApp()
