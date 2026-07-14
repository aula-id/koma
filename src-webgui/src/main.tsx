import React from 'react'
import ReactDOM from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import './styles.css'

// webkitgtk font-loading race: if text paints before the bundled KomaMono
// (JetBrains Mono) faces finish loading, it renders with a fallback face —
// and webkitgtk often never repaints once the real font lands
// (font-display: swap repaint bug). Gate the first render on the faces
// being loaded so static regions (e.g. the PLAN todo list, which never
// re-renders) never get stuck with the wrong face.
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
  Promise.all(fonts)
    .then(() => document.fonts.ready)
    .catch(() => undefined)
    .then(() => renderApp())
} else {
  renderApp()
}
