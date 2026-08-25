import React from 'react'
import ReactDOM from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import './styles.css'

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
