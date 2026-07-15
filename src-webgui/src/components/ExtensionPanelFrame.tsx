import { useEffect, useRef } from 'react'
import { registerPanelFrame, unregisterPanelFrame, PANEL_ORIGIN } from '../lib/panelBridge'

// Renders one extension panel iframe and keeps the panelBridge registry
// (see lib/panelBridge.ts) pointed at its live `contentWindow`. Re-registers
// on every `onLoad` — a reload (or the panel navigating itself) hands out a
// fresh `contentWindow` proxy, so the registry entry must be refreshed each
// time, not just once on mount.
export function ExtensionPanelFrame({
  extId,
  panelId,
  title,
}: {
  extId: string
  panelId: string
  title: string
}) {
  const iframeRef = useRef<HTMLIFrameElement>(null)

  useEffect(() => {
    return () => {
      unregisterPanelFrame(extId, panelId)
    }
  }, [extId, panelId])

  return (
    // The panel loads from `${PANEL_ORIGIN}/<id>/index.html` — a SEPARATE origin
    // from the host chrome (served by `handle_extension_request` off the
    // installed extension's own `ui/` dir), so the panel's own page can never
    // script this one. `PANEL_ORIGIN` is `koma://extension` on macOS/Linux, but
    // `http://koma.extension` on Windows: WebView2/Chromium can't register a real
    // custom scheme, so a JS-set `src` of `koma://extension/...` never loads
    // (blank frame) — wry only intercepts the http fake-domain form it maps the
    // scheme to (see lib/panelBridge.ts PANEL_ORIGIN). No `sandbox` attribute:
    // the extension origin isolation already provides the security boundary, and
    // a restrictive `sandbox` would block the custom scheme from loading at all.
    <iframe
      ref={iframeRef}
      src={`${PANEL_ORIGIN}/${extId}/index.html`}
      title={title}
      className="h-full w-full border-0"
      onLoad={() => {
        const win = iframeRef.current?.contentWindow
        if (win) registerPanelFrame(extId, panelId, win)
      }}
    />
  )
}
